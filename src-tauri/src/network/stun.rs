use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::UdpSocket;

const STUN_SERVERS: [&str; 3] = [
    "stun.l.google.com:19302",
    "stun.cloudflare.com:3478",
    "stun1.l.google.com:19302",
];

const MAGIC_COOKIE: [u8; 4] = [0x21, 0x12, 0xA4, 0x42];

fn parse_xor_mapped_address(v: &[u8], txid: &[u8]) -> Option<SocketAddr> {
    if v.len() < 4 {
        return None;
    }
    let family = v[1];
    let port = u16::from_be_bytes([v[2], v[3]]) ^ 0x2112;
    match family {
        0x01 if v.len() >= 8 => {
            let ip = [
                v[4] ^ MAGIC_COOKIE[0],
                v[5] ^ MAGIC_COOKIE[1],
                v[6] ^ MAGIC_COOKIE[2],
                v[7] ^ MAGIC_COOKIE[3],
            ];
            Some(SocketAddr::from((ip, port)))
        }
        0x02 if v.len() >= 20 => {
            let mut ip = [0u8; 16];
            for i in 0..4 {
                ip[i] = v[4 + i] ^ MAGIC_COOKIE[i];
            }
            for i in 0..12 {
                ip[4 + i] = v[8 + i] ^ txid[i];
            }
            Some(SocketAddr::from((ip, port)))
        }
        _ => None,
    }
}

pub async fn mapped_addr(sock: &UdpSocket) -> Result<SocketAddr, String> {
    let mut last_err = String::from("no STUN server reachable");
    for srv in STUN_SERVERS {
        let srv_addr = match tokio::net::lookup_host(srv).await {
            Ok(mut it) => it.find(|a| a.is_ipv4()),
            Err(e) => {
                last_err = format!("resolve {srv}: {e}");
                continue;
            }
        };
        let Some(srv_addr) = srv_addr else {
            last_err = format!("no address for {srv}");
            continue;
        };

        let mut msg = stun::message::Message::new();
        msg.set_type(stun::message::BINDING_REQUEST);
        if let Err(e) = msg.new_transaction_id() {
            last_err = format!("txid: {e}");
            continue;
        }
        msg.encode();
        let txid = msg.transaction_id.0;
        let mut raw = Vec::new();
        if let Err(e) = msg.write_to(&mut raw) {
            last_err = format!("encode: {e}");
            continue;
        }

        if let Err(e) = sock.send_to(&raw, srv_addr).await {
            last_err = format!("send {srv}: {e}");
            continue;
        }

        let outcome = tokio::time::timeout(Duration::from_secs(4), async {
            let mut buf = [0u8; 2048];
            loop {
                let (n, _) = sock.recv_from(&mut buf).await.map_err(|e| e.to_string())?;
                if !stun::message::is_message(&buf[..n]) {
                    continue;
                }
                let mut resp = stun::message::Message::new();
                resp.write(&buf[..n]).map_err(|e| e.to_string())?;
                if resp.transaction_id.0 != txid {
                    continue;
                }
                if resp.typ == stun::message::BINDING_ERROR {
                    if let Ok(v) = resp.get(stun::attributes::ATTR_ERROR_CODE) {
                        let code = u16::from_be_bytes([v[2], v[3]]);
                        return Err(format!("STUN {srv} error {code}"));
                    }
                    return Err(format!("STUN {srv} error response"));
                }
                if resp.typ == stun::message::BINDING_SUCCESS {
                    let v = resp
                        .get(stun::attributes::ATTR_XORMAPPED_ADDRESS)
                        .map_err(|e| format!("no xor-mapped address: {e}"))?;
                    let addr = parse_xor_mapped_address(&v, &txid)
                        .ok_or_else(|| String::from("malformed xor-mapped address"))?;
                    return Ok(addr);
                }
            }
        })
        .await;

        match outcome {
            Ok(Ok(addr)) => return Ok(addr),
            Ok(Err(e)) => last_err = e,
            Err(_) => last_err = format!("timeout on {srv}"),
        }
    }
    Err(last_err)
}