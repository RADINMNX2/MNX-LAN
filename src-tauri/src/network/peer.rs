use std::net::SocketAddr;

use super::super::lobby::protocol::PeerInfo;

#[derive(Clone, Debug)]
pub struct PeerRuntime {
    pub info: PeerInfo,
    pub endpoint: Option<SocketAddr>,
    pub last_seen: u64,
    pub established: bool,
}

pub fn new_udp_socket(bind: SocketAddr) -> Result<tokio::net::UdpSocket, String> {
    let sock = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .map_err(|e| format!("socket create: {e}"))?;
    let _ = sock.set_tos(0x2E);
    let _ = sock.set_reuse_address(true);
    sock.bind(&bind.into()).map_err(|e| format!("bind {bind}: {e}"))?;
    sock.set_nonblocking(true).map_err(|e| e.to_string())?;
    let std_sock: std::net::UdpSocket = sock.into();
    tokio::net::UdpSocket::from_std(std_sock).map_err(|e| format!("tokio socket: {e}"))
}