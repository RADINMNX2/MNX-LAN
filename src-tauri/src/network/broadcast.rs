use std::net::Ipv4Addr;

use super::super::lobby::protocol::ip_v4;

pub const GAME_UDP_PORTS: &[u16] = &[
    27015, 27016, 27020, 27021, 19132, 19133, 6112, 2302, 2303, 7777, 25565, 25575, 34197, 4380,
    14801, 38999,
];

pub const SUBNET_BROADCAST_V4: Ipv4Addr = Ipv4Addr::new(10, 88, 255, 255);
pub const LIMITED_BROADCAST_V4: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 255);

pub fn is_broadcast_candidate(pkt: &[u8]) -> Option<(u16, u16, Ipv4Addr, Ipv4Addr, usize)> {
    if pkt.len() < 20 {
        return None;
    }
    if pkt[0] >> 4 != 4 {
        return None;
    }
    let ihl = ((pkt[0] & 0x0f) as usize) * 4;
    if pkt.len() < ihl + 8 {
        return None;
    }
    if pkt[9] != 17 {
        return None;
    }
    let src = Ipv4Addr::new(pkt[12], pkt[13], pkt[14], pkt[15]);
    let dst = Ipv4Addr::new(pkt[16], pkt[17], pkt[18], pkt[19]);
    let destination_is_broadcast = dst == LIMITED_BROADCAST_V4 || dst == SUBNET_BROADCAST_V4;
    if !destination_is_broadcast {
        return None;
    }
    let src_port = u16::from_be_bytes([pkt[ihl], pkt[ihl + 1]]);
    let dst_port = u16::from_be_bytes([pkt[ihl + 2], pkt[ihl + 3]]);
    let blocked = matches!(dst_port, 67 | 68 | 1900 | 5353 | 3702) || dst_port == 5355 || dst_port == 546;
    if GAME_UDP_PORTS.contains(&dst_port) || (!blocked && src_port != 0) {
        return Some((src_port, dst_port, src, dst, ihl));
    }
    None
}

pub fn packet_src_dst(pkt: &[u8]) -> Option<(u32, u32)> {
    if pkt.len() < 20 || pkt[0] >> 4 != 4 {
        return None;
    }
    let src = ip_v4(pkt[12], pkt[13], pkt[14], pkt[15]);
    let dst = ip_v4(pkt[16], pkt[17], pkt[18], pkt[19]);
    Some((src, dst))
}

pub fn rebuild_broadcast(pkt: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.extend_from_slice(pkt);
    if out.len() < 20 || out[0] >> 4 != 4 {
        return;
    }
    let ihl = ((out[0] & 0x0f) as usize) * 4;
    if out.len() < ihl + 8 {
        return;
    }
    out[16] = 255;
    out[17] = 255;
    out[18] = 255;
    out[19] = 255;
    if out[9] == 17 && out.len() >= ihl + 8 {
        out[ihl + 6] = 0;
        out[ihl + 7] = 0;
    }
    let sum = ip_checksum(&out[0..ihl]);
    out[10] = (sum >> 8) as u8;
    out[11] = (sum & 0xff) as u8;
}

pub fn ip_checksum(header: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < header.len() {
        sum += ((header[i] as u32) << 8) | header[i + 1] as u32;
        i += 2;
    }
    if header.len() % 2 == 1 {
        sum += (header[header.len() - 1] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}