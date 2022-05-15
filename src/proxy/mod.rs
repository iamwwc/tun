use core::fmt;
use std::{
    io,
    net::{IpAddr, SocketAddr},
    os::unix::prelude::{FromRawFd, IntoRawFd}, sync::Arc, convert::TryFrom, fmt::Display, ops::Add,
};

use anyhow::{
    anyhow
};
use async_trait::async_trait;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpSocket, UdpSocket, TcpStream}, sync::RwLock,
};

use crate::{app::DnsClient, Context};

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

pub struct DomainSession {
    name: String,
    port: u16,
}

#[derive(Debug, Clone)]
pub enum Address {
    Domain(String, u16),
    Ip(SocketAddr)
}
impl Address {
    pub fn port(&self) -> u16 {
        match self {
            Address::Domain(_, port) => *port,
            Address::Ip(addr) => addr.port()
        }
    }
    pub fn host(&self) -> String {
        match self {
            Address::Domain(n,_ ) => n.clone(),
            Address::Ip(addr) => addr.to_string()
        }
    }
}


impl Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let str = match self {
            Address::Domain(name, port) => format!("{}:{}", name, port),
            Address::Ip(addr) => addr.to_string()
        };
        write!(f, "{}", str)
    }
}

impl TryFrom<(String, u16)> for Address {
    type Error = io::Error;
    fn try_from(value: (String, u16)) -> Result<Self, Self::Error> {
        Ok(Address::Domain(value.0, value.1))
    }
}

impl Into<String> for Address {
    fn into(self) -> String {
        match self {
            Address::Domain(name, port) => {
                format!("{}:{}",name, port)
            },
            Address::Ip(addr) => addr.to_string()
        }
    }
}

impl From<String> for Address {
    fn from(s: String) -> Self {
        let address = match s.parse::<SocketAddr>(){
            Ok(res) => Self::Ip(res),
            Err(err) => {
                // maybe a domain name
                // if it's a bad domain:port, exception will raise when connect to it
                let parts: Vec<&str> = s.split(':').collect();
                let port = u16::from_str_radix(*parts.get(0).unwrap_or(&"0"), 10).unwrap();
                Self::Domain(s, port)
            }
        };
        address
    }
}


#[derive(Debug, Clone)]
pub enum Network {
    TCP,
    UDP
}
// connection session
#[derive(Debug, Clone)]
pub struct Session {
    pub destination: Address,
    // 连接到本地代理服务器的remote
    // local_peer <=> tunnel
    pub local_peer: SocketAddr,
    
    pub network: Network
}
impl Session {
    pub fn port (&self) -> u16{
        match self.destination {
            Address::Domain(_, p) => p,
            Address::Ip(addr) => addr.port()
        }
    }
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
    Stream(TcpStream, Session),
    Datagram(UdpSocket, Session),
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
    async fn handle(&self, session: Session, stream: TcpStream) -> io::Result<InboundResult>;
}

#[async_trait]
pub trait UdpInboundHandlerTrait {
    async fn handle(&self, session: Session, socket: tokio::net::UdpSocket) -> io::Result<InboundResult>;
}

// OUTBOUND

pub enum OutboundConnect {
    // used by socks, shadowsocks ... proxy protocol
    // String can be socketaddr or domain name
    Proxy(String, u16),
    // direct protocol
    Direct,
    // drop
    Drop
}

#[async_trait]
pub trait TcpOutboundHandlerTrait: Send + Sync + Unpin {
    // remote addr should be connected directly
    // no proxy involved
    // fn remote_addr(&self) -> OutboundConnect;
    async fn handle(&self, ctx: Arc<Context>, sess: &Session) -> Result<TcpStream, Error>;
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("connect to {0}:{1} failed")]
    ConnectError(String, u16)
}

#[async_trait]
pub trait UdpOutboundHandlerTrait: Send + Sync + Unpin {
    async fn handle(&self, ctx: Arc<Context>, sess: &Session) -> Result<UdpSocket, Error>;
}

pub type AnyTcpOutboundHandler = Arc<dyn TcpOutboundHandlerTrait>;
pub type AnyUdpOutboundHandler = Arc<dyn UdpOutboundHandlerTrait>;
pub trait AnyOutboundHandlerTrait: TcpOutboundHandlerTrait + UdpOutboundHandlerTrait + Unpin + Send + Sync {}
pub type AnyOutboundHandler = Arc<dyn AnyOutboundHandlerTrait>;

pub struct OutboundHandler {
    pub tag: String,
    pub tcp_handler: Option<AnyTcpOutboundHandler>,
    pub udp_handler: Option<AnyUdpOutboundHandler>,
}

impl OutboundHandler {
    pub fn new(tag: String, tcp: Option<AnyTcpOutboundHandler>, udp: Option<AnyUdpOutboundHandler>) -> OutboundHandler {
        OutboundHandler { tag , tcp_handler: tcp, udp_handler: udp }
    }
}

pub trait StreamWrapperTrait: AsyncRead + AsyncWrite + Send + Sync + Unpin{}
impl<T> StreamWrapperTrait for T where T: AsyncRead + AsyncWrite + Send + Sync + Unpin {}


pub async fn connect_to_remote_tcp(dns_client:Arc<RwLock<DnsClient>>, addr: Address) -> anyhow::Result<TcpStream>{
    let socket_addr = name_to_socket_addr(dns_client, addr).await?;
    // 这样可以
    Ok(TcpStream::connect(socket_addr).await?)
    // 但下面不行
    // TcpStream::connect(socket_addr).await
    // 原因是 ? 进行 type conversion, anyhow::Result 实现了 from io::Error 转换
    // https://stackoverflow.com/a/62241599/7529562
}

pub async fn name_to_socket_addr(dns_client: Arc<RwLock<DnsClient>>, addr: Address) -> anyhow::Result<SocketAddr> {
    let socket_addr = match addr {
        Address::Domain(name, port) => {
            match dns_client.read().await.lookup(&format!("{}:{}", name, port)).await {
                Ok(ips) => {
                    // TODO connect to multiple ips
                    let ip = if let Some(ip) = ips.get(0) {
                        ip
                    }else {
                        return Err(anyhow!("dns not ip found"))
                    };
                    SocketAddr::new(ip.clone(), port)
                },
                Err(e) => {
                    return Err(e)
                }
            }
        },
        Address::Ip(addr) => addr
    };
    Ok(socket_addr)
}

pub async fn connect_to_remote_udp(dns_client: Arc<RwLock<DnsClient>>, local: SocketAddr, peer: Address) -> anyhow::Result<UdpSocket> {
    let socket = UdpSocket::bind(local).await?;
    let socket_addr = name_to_socket_addr(dns_client, peer).await?;
    UdpSocket::connect(&socket, socket_addr).await?;
    Ok(socket)
}