use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub const DEFAULT_BROADCAST_PORTS: [u16; 3] = [27015, 19132, 6112];

const DEDUP_TTL: Duration = Duration::from_millis(500);
const UNICAST_BROADCAST_PKT: u32 = 0xFFFFFFFF;

fn fnv1a(buf: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in buf {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[derive(Default)]
struct DedupCache {
    entries: HashMap<u64, Instant>,
}

impl DedupCache {
    fn is_fresh(&mut self, key: u64) -> bool {
        let now = Instant::now();
        self.entries.retain(|_, t| now.duration_since(*t) < DEDUP_TTL);
        if self.entries.contains_key(&key) {
            return false;
        }
        self.entries.insert(key, now);
        true
    }
}

fn classify_broadcast(buf: &[u8]) -> Option<(u16, u16)> {
    if buf.len() < 28 {
        return None;
    }
    match buf[0] >> 4 {
        4 => {
            let ihl = ((buf[0] & 0x0F) as usize) * 4;
            if buf.len() < ihl + 8 || buf[9] != 17 {
                return None;
            }
            let dst: [u8; 4] = [buf[16], buf[17], buf[18], buf[19]];
            let dst_u32 = u32::from_be_bytes(dst);
            let is_broadcast = dst_u32 == UNICAST_BROADCAST_PKT || (dst[0] == 10 && dst[3] == 255);
            if !is_broadcast {
                return None;
            }
            let src_port = u16::from_be_bytes([buf[ihl], buf[ihl + 1]]);
            let dst_port = u16::from_be_bytes([buf[ihl + 2], buf[ihl + 3]]);
            Some((src_port, dst_port))
        }
        _ => None,
    }
}

pub struct BroadcastReplicator {
    allowed_ports: HashSet<u16>,
    pub fanout_cap: usize,
    cache: DedupCache,
}

impl BroadcastReplicator {
    pub fn new(custom_ports: &[u16]) -> Self {
        let mut allowed = HashSet::new();
        for p in DEFAULT_BROADCAST_PORTS.iter().chain(custom_ports) {
            allowed.insert(*p);
        }
        BroadcastReplicator {
            allowed_ports: allowed,
            fanout_cap: 64,
            cache: DedupCache::default(),
        }
    }

    pub fn set_custom_ports(&mut self, ports: &[u16]) {
        for p in ports {
            self.allowed_ports.insert(*p);
        }
    }

    pub fn should_replicate(&mut self, ip_packet: &[u8]) -> Option<Vec<u8>> {
        let (src_port, dst_port) = classify_broadcast(ip_packet)?;
        if !self.allowed_ports.contains(&src_port) && !self.allowed_ports.contains(&dst_port) {
            return None;
        }
        let key = fnv1a(ip_packet);
        if !self.cache.is_fresh(key) {
            return None;
        }
        Some(ip_packet.to_vec())
    }
}