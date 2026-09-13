use crate::lobby::protocol::{decode_frame, encode_frame, FrameKind, KIND_PING};
use crate::network::peer::bind_tos_socket;
use crate::network::stun;

pub async fn run() -> bool {
    let mut ok = true;

    let frame = encode_frame(KIND_PING, &[]);
    let parsed = decode_frame(&frame);
    match parsed {
        Some((FrameKind::Ping, p)) if p.is_empty() => {
            println!("[selftest] frame roundtrip ......... PASS");
        }
        _ => {
            println!("[selftest] frame roundtrip ......... FAIL");
            ok = false;
        }
    }

    let sock = match bind_tos_socket() {
        Ok(s) => s,
        Err(e) => {
            println!("[selftest] tos socket ............... FAIL ({e})");
            return false;
        }
    };
    let local = sock
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "unknown".into());
    println!("[selftest] tos socket (DSCP=EF) ...... PASS ({local})");

    match stun::mapped_addr(&sock).await {
        Ok(addr) => println!("[selftest] stun reachability .......... PASS (mapped {addr})"),
        Err(e) => {
            println!("[selftest] stun reachability .......... FAIL ({e})");
            ok = false;
        }
    }

    print!(
        "[selftest] wintun dll embedded ......... {} bytes\n",
        crate::network::adapter::embedded_dll_len()
    );

    ok
}