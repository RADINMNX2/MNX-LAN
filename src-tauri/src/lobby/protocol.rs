use serde::{Deserialize, Serialize};

pub const MAGIC: [u8; 4] = *b"MNXL";
pub const FRAME_VERSION: u8 = 1;

pub const KIND_CTRL: u8 = b'C';
pub const KIND_DATA: u8 = b'D';
pub const KIND_PING: u8 = b'P';
pub const KIND_PONG: u8 = b'Q';

pub const HEADER_LEN: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfoWire {
    pub name: String,
    pub virtual_ip: String,
    pub local_addr: String,
    pub mapped_addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", content = "d", rename_all = "snake_case")]
pub enum CtrlMsg {
    JoinRequest {
        room: String,
        name: String,
        mapped_addr: String,
    },
    JoinAck {
        room: String,
        host: String,
        vip: String,
        peers: Vec<PeerInfoWire>,
    },
    PeerJoined {
        peer: PeerInfoWire,
    },
    PeerGone {
        name: String,
    },
    Heartbeat {
        room: String,
        name: String,
    },
}

pub fn encode_frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(&MAGIC);
    out.push(FRAME_VERSION);
    out.push(kind);
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    Ctrl,
    Data,
    Ping,
    Pong,
    Unknown,
}

pub fn decode_frame(buf: &[u8]) -> Option<(FrameKind, &[u8])> {
    if buf.len() < HEADER_LEN || buf[..4] != MAGIC || buf[4] != FRAME_VERSION {
        return None;
    }
    let len = u16::from_be_bytes([buf[6], buf[7]]) as usize;
    if buf.len() < HEADER_LEN + len {
        return None;
    }
    let kind = match buf[5] {
        KIND_CTRL => FrameKind::Ctrl,
        KIND_DATA => FrameKind::Data,
        KIND_PING => FrameKind::Ping,
        KIND_PONG => FrameKind::Pong,
        _ => FrameKind::Unknown,
    };
    Some((kind, &buf[HEADER_LEN..HEADER_LEN + len]))
}

pub fn ctrl_frame(msg: &CtrlMsg) -> Result<Vec<u8>, String> {
    let json = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
    Ok(encode_frame(KIND_CTRL, &json))
}

pub fn data_frame(ip_packet: &[u8]) -> Vec<u8> {
    encode_frame(KIND_DATA, ip_packet)
}

pub fn ping_frame(ts_ms: u64) -> Vec<u8> {
    encode_frame(KIND_PING, &ts_ms.to_be_bytes())
}

pub fn pong_frame(ts_ms: u64) -> Vec<u8> {
    encode_frame(KIND_PONG, &ts_ms.to_be_bytes())
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn ts_from_payload(payload: &[u8]) -> u64 {
    if payload.len() != 8 {
        return 0;
    }
    u64::from_be_bytes(payload[..8].try_into().unwrap_or([0; 8]))
}