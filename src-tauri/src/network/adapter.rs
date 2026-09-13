use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;

pub const ADAPTER_NAME: &str = "MNX-LAN";
pub const TUNNEL_TYPE: &str = "RADIN MNX Virtual LAN";
pub const MTU: usize = 1400;
pub const SUBNET_MASK: Ipv4Addr = Ipv4Addr::new(255, 255, 0, 0);

pub struct LoadedDll(pub wintun::Wintun);
unsafe impl Send for LoadedDll {}
unsafe impl Sync for LoadedDll {}

pub struct Tun {
    pub adapter: Arc<wintun::Adapter>,
    pub session: Arc<wintun::Session>,
    pub dll: LoadedDll,
    pub running: Arc<AtomicBool>,
    pub reader: Option<std::thread::JoinHandle<()>>,
}

pub fn load_wintun(candidates: &[PathBuf]) -> Result<(PathBuf, LoadedDll), String> {
    for path in candidates {
        if path.is_file() {
            let path_str = path.to_str().ok_or_else(|| "Invalid wintun.dll path".to_string())?;
            let dll = unsafe { wintun::load_from_path(path_str) }
                .map_err(|e| format!("Failed to load wintun.dll from {}: {:?}", path_str, e))?;
            return Ok((path.clone(), LoadedDll(dll)));
        }
    }
    Err("wintun.dll not found. Place it next to MNX-LAN.exe or install a Wintun-shipping app.".into())
}

pub fn ensure_adapter(dll: &wintun::Wintun) -> Result<Arc<wintun::Adapter>, String> {
    let adapter = match wintun::Adapter::open(dll, ADAPTER_NAME) {
        Ok(a) => a,
        Err(_) => wintun::Adapter::create(dll, ADAPTER_NAME, TUNNEL_TYPE, None)
            .map_err(|e| format!("Failed to create Wintun adapter: {:?}", e))?,
    };
    Ok(adapter)
}

pub fn configure(adapter: &wintun::Adapter, vip: Ipv4Addr) -> Result<(), String> {
    adapter
        .set_network_addresses_tuple(IpAddr::V4(vip), IpAddr::V4(SUBNET_MASK), None)
        .map_err(|e| format!("Failed to assign virtual IP {} (requires Administrator): {:?}", vip, e))?;
    adapter
        .set_mtu(MTU)
        .map_err(|e| format!("Failed to set MTU: {:?}", e))?;
    Ok(())
}

pub fn start_session(adapter: &Arc<wintun::Adapter>) -> Result<Arc<wintun::Session>, String> {
    adapter
        .start_session(wintun::MAX_RING_CAPACITY)
        .map(Arc::new)
        .map_err(|e| format!("Failed to start Wintun session: {:?}", e))
}

pub fn spawn_read_loop(
    session: Arc<wintun::Session>,
    tx: mpsc::Sender<Vec<u8>>,
    running: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while running.load(Ordering::Relaxed) {
            match session.receive_blocking() {
                Ok(packet) => {
                    let bytes = packet.bytes().to_vec();
                    if tx.blocking_send(bytes).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    })
}

pub fn write_ip(session: &Arc<wintun::Session>, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() > 1500 {
        return Ok(());
    }
    let mut packet = session
        .allocate_send_packet(bytes.len() as u16)
        .map_err(|e| format!("TUN send allocate failed: {:?}", e))?;
    packet.bytes_mut().copy_from_slice(bytes);
    session.send_packet(packet);
    Ok(())
}