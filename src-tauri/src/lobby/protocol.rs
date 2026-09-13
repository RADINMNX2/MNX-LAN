use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerInfo {
    pub id: String,
    pub name: String,
    pub vip: String,
    pub public: String,
    pub rtt_ms: u64,
    pub p2p: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LobbyMsg {
    Ping { tok: u64, ts: u64 },
    Pong { tok: u64, ts: u64, id: String },
    Hello { room: String, name: String, nonce: u64 },
    Welcome { room: String, vip: String, host_name: String },
    PeerList { peers: Vec<PeerInfo> },
    PeerJoin { peer: PeerInfo },
    PeerLeave { id: String },
    PeerPoke { peer: PeerInfo },
    Punch { from: String },
    Bye { id: String },
}

pub const FRAME_MAGIC: u32 = 0x4D4E5821;
pub const FRAME_VERSION: u8 = 1;
pub const F_BROADCAST: u8 = 0x01;
pub const F_CONTROL: u8 = 0x02;
pub const F_TUNNEL: u8 = 0x04;
pub const FRAME_HEADER: usize = 18;

pub const SIGNAL_PORT: u16 = 38888;
pub const MEDIA_SUBNET_A: u8 = 10;
pub const MEDIA_SUBNET_B: u8 = 88;
pub const HOST_VIP: u32 = ip_v4(MEDIA_SUBNET_A, MEDIA_SUBNET_B, 0, 1);

pub fn ip_v4(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) << 24 | (b as u32) << 16 | (c as u32) << 8 | d as u32
}

pub fn v4_to_u32(ip: std::net::Ipv4Addr) -> u32 {
    let o = ip.octets();
    ip_v4(o[0], o[1], o[2], o[3])
}

pub fn u32_to_v4(v: u32) -> std::net::Ipv4Addr {
    std::net::Ipv4Addr::new((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub fn encode_frame(flags: u8, src: u32, dst: u32, payload: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(FRAME_HEADER + payload.len());
    out.extend_from_slice(&FRAME_MAGIC.to_le_bytes());
    out.push(FRAME_VERSION);
    out.push(flags);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&src.to_le_bytes());
    out.extend_from_slice(&dst.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    out.extend_from_slice(payload);
}

pub fn decode_frame(buf: &[u8]) -> Option<(u8, u32, u32, &[u8])> {
    if buf.len() < FRAME_HEADER {
        return None;
    }
    let magic = u32::from_le_bytes(buf[0..4].try_into().ok()?);
    if magic != FRAME_MAGIC {
        return None;
    }
    if buf[4] != FRAME_VERSION {
        return None;
    }
    let flags = buf[5];
    let src = u32::from_le_bytes(buf[8..12].try_into().ok()?);
    let dst = u32::from_le_bytes(buf[12..16].try_into().ok()?);
    let len = u16::from_le_bytes(buf[16..18].try_into().ok()?) as usize;
    if buf.len() < FRAME_HEADER + len {
        return None;
    }
    Some((flags, src, dst, &buf[FRAME_HEADER..FRAME_HEADER + len]))
}

pub fn fresh_id() -> String {
    use rand::Rng;
    format!("mx-{:012x}", rand::thread_rng().gen::<u64>() & 0xffff_ffff_ffff)
}

pub fn room_code() -> String {
    use rand::seq::SliceRandom;
    use rand::thread_rng;
    const CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = thread_rng();
    let mut code = String::from("MNX-");
    for _ in 0..4 {
        let c = CHARS.choose(&mut rng).copied().unwrap_or(b'A');
        code.push(c as char);
    }
    code
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}