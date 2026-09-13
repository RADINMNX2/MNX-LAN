use std::collections::HashMap;
use std::net::Ipv4Addr;

use super::protocol::{fresh_id, ip_v4, room_code, PeerInfo};

#[derive(Clone, Debug)]
pub struct Room {
    pub code: String,
    pub host_id: String,
    pub host_name: String,
    pub host_public: String,
    pub self_peer: PeerInfo,
    pub next_host_byte: u32,
    pub peers: HashMap<String, PeerInfo>,
}

impl Room {
    pub fn new(name: String, public: String, vip: String) -> Self {
        let id = fresh_id();
        let self_peer = PeerInfo {
            id: id.clone(),
            name,
            vip,
            public: public.clone(),
            rtt_ms: 0,
            p2p: false,
        };
        Room {
            code: room_code(),
            host_id: id,
            host_name: self_peer.name.clone(),
            host_public: public,
            self_peer,
            next_host_byte: 2,
            peers: HashMap::new(),
        }
    }

    pub fn alloc_vip(&mut self) -> Option<String> {
        while self.next_host_byte <= 254 {
            let b = self.next_host_byte;
            self.next_host_byte += 1;
            let vip_u32 = ip_v4(10, 88, 0, b as u8);
            let vip = Ipv4Addr::from(vip_u32).to_string();
            let taken = self.peers.values().any(|p| p.vip == vip) || self.self_peer.vip == vip;
            if !taken {
                return Some(vip);
            }
        }
        None
    }
}