use crate::lobby::protocol::now_ms;

pub fn gen_room_id() -> String {
    let seed = now_ms() ^ (std::process::id() as u64).wrapping_mul(0x9E3779B97F4A7C15);
    let mut rng = seed | 1;
    let mut id = String::from("MNX-");
    for _ in 0..4 {
        let idx = (rng % 36) as usize;
        id.push(b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"[idx] as char);
        rng = rng
            .wrapping_mul(1664525)
            .wrapping_add(1013904223);
    }
    id
}

#[derive(Clone, Debug)]
pub struct Room {
    pub id: String,
    pub host_name: String,
    pub peers: std::collections::HashMap<String, RoomPeer>,
    next_vip: u8,
}

pub const HOST_VIP: &str = "10.88.0.1";
pub const VIRTUAL_NET: &str = "10.88.0.0";
pub const VIRTUAL_MASK: &str = "255.255.0.0";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RoomPeer {
    pub name: String,
    pub virtual_ip: String,
    pub local_addr: String,
    pub mapped_addr: String,
}

impl Room {
    pub fn create(host_name: &str) -> Self {
        Room {
            id: gen_room_id(),
            host_name: host_name.to_string(),
            peers: std::collections::HashMap::new(),
            next_vip: 2,
        }
    }

    pub fn next_virtual_ip(&mut self) -> String {
        if self.next_vip > 254 {
            return HOST_VIP.to_string();
        }
        let ip = format!("10.88.0.{}", self.next_vip);
        self.next_vip += 1;
        ip
    }
}