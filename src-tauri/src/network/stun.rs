use std::net::{SocketAddr, SocketAddrV4};

use tokio::net::UdpSocket;

pub const STUN_SERVERS: &[&str] = &[
    "stun.l.google.com:19302",
    "stun1.l.google.com:19302",
    "stun.cloudflare.com:3478",
];

const STUN_MAGIC_COOKIE: u32 = 0x2112_A442;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

pub async fn discover_public_addr(sock: &UdpSocket) -> Option<SocketAddr> {
    for server in STUN_SERVERS {
        let Ok(server) = server.parse::<SocketAddr>() else {
            continue;
        };
        if let Ok(mapped) = request_mapped_addr(sock, server).await {
            return Some(mapped);
        }
    }
    None
}

async fn request_mapped_addr(sock: &UdpSocket, server: SocketAddr) -> Result<SocketAddr, ()> {
    use rand::RngCore;
    let mut tid = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut tid);
    let request = build_binding_request(&tid);
    sock.send_to(&request, server).await.map_err(|_| ())?;

    let mut buf = [0u8; 512];
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(2);
    loop {
        let remain = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remain.is_zero() {
            return Err(());
        }
        let (n, _) = tokio::time::timeout(remain, sock.recv_from(&mut buf))
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
        if let Some(addr) = parse_xor_mapped(&buf[..n], &tid) {
            return Ok(addr);
        }
    }
}

fn build_binding_request(tid: &[u8; 12]) -> [u8; 20] {
    let mut m = [0u8; 20];
    m[0] = 0x00;
    m[1] = 0x01;
    m[4] = (STUN_MAGIC_COOKIE >> 24) as u8;
    m[5] = (STUN_MAGIC_COOKIE >> 16) as u8;
    m[6] = (STUN_MAGIC_COOKIE >> 8) as u8;
    m[7] = STUN_MAGIC_COOKIE as u8;
    m[8..20].copy_from_slice(tid);
    m
}

fn parse_xor_mapped(buf: &[u8], tid: &[u8; 12]) -> Option<SocketAddr> {
    if buf.len() < 20 {
        return None;
    }
    let msg_type = u16::from_be_bytes([buf[0], buf[1]]);
    if msg_type != 0x0101 {
        return None;
    }
    let msg_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    if buf.len() < 20 + msg_len {
        return None;
    }
    let cookie = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    if cookie != STUN_MAGIC_COOKIE {
        return None;
    }
    if &buf[8..20] != tid {
        return None;
    }
    let mut i = 20;
    while i + 4 <= buf.len() {
        let atype = u16::from_be_bytes([buf[i], buf[i + 1]]);
        let alen = u16::from_be_bytes([buf[i + 2], buf[i + 3]]) as usize;
        let value = i + 4;
        if value + alen > buf.len() {
            break;
        }
        if atype == ATTR_XOR_MAPPED_ADDRESS && alen >= 8 {
            let family = buf[value + 1];
            if family == 0x01 {
                let xport = u16::from_be_bytes([buf[value + 2], buf[value + 3]]);
                let port = xport ^ (STUN_MAGIC_COOKIE >> 16) as u16;
                let raddr = u32::from_be_bytes([
                    buf[value + 4],
                    buf[value + 5],
                    buf[value + 6],
                    buf[value + 7],
                ]);
                let addr = raddr ^ STUN_MAGIC_COOKIE;
                return Some(SocketAddr::V4(SocketAddrV4::new(
                    std::net::Ipv4Addr::from(addr),
                    port,
                )));
            }
        }
        i += 4 + alen;
    }
    None
}