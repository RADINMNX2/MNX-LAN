use std::net::SocketAddr;
use std::time::Instant;

use tokio::net::UdpSocket;

pub const DSCP_EF: u32 = 0xB8;

#[derive(Clone)]
pub struct Peer {
    pub name: String,
    pub virtual_ip: String,
    pub local_addr: String,
    pub mapped_addr: String,
    pub endpoint: SocketAddr,
    pub rtt_ms: Option<u64>,
    pub last_seen: Instant,
}

impl Peer {
    pub fn new(
        name: String,
        virtual_ip: String,
        local_addr: String,
        mapped_addr: String,
        endpoint: SocketAddr,
    ) -> Self {
        Peer {
            name,
            virtual_ip,
            local_addr,
            mapped_addr,
            endpoint,
            rtt_ms: None,
            last_seen: Instant::now(),
        }
    }
}

pub fn bind_tos_socket() -> Result<UdpSocket, String> {
    use socket2::{Domain, Protocol, SockAddr, Socket, Type};

    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .map_err(|e| e.to_string())?;
    sock.set_tos(DSCP_EF).map_err(|e| e.to_string())?;
    sock.set_reuse_address(true).map_err(|e| e.to_string())?;
    sock.set_nonblocking(true).map_err(|e| e.to_string())?;

    let any: SocketAddr = "0.0.0.0:0".parse().map_err(|e| e.to_string())?;
    sock.bind(&SockAddr::from(any)).map_err(|e| e.to_string())?;

    let std_sock: std::net::UdpSocket = sock.into();
    UdpSocket::from_std(std_sock).map_err(|e| e.to_string())
}