use std::sync::Arc;

use crate::lobby::room::VIRTUAL_MASK;

const EMBEDDED_DLL: &[u8] = include_bytes!("../../assets/wintun/x64/wintun.dll");

#[cfg(target_os = "windows")]
pub fn is_elevated() -> bool {
    use windows_sys::Win32::UI::Shell::IsUserAnAdmin;
    unsafe { IsUserAnAdmin() != 0 }
}

#[cfg(not(target_os = "windows"))]
pub fn is_elevated() -> bool {
    true
}

pub fn embedded_dll_len() -> usize {
    EMBEDDED_DLL.len()
}

fn dll_file() -> Result<std::path::PathBuf, String> {
    let dir = std::env::temp_dir().join("mnx-lan");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("wintun.dll");
    if !path.exists() || std::fs::metadata(&path).map(|m| m.len() != EMBEDDED_DLL.len() as u64).unwrap_or(true) {
        std::fs::write(&path, EMBEDDED_DLL).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

pub struct TunAdapter {
    session: Arc<wintun::Session>,
    _adapter: Arc<wintun::Adapter>,
}

impl TunAdapter {
    pub fn create() -> Result<Self, String> {
        let dll = dll_file()?;
        let wintun = unsafe { wintun::load_from_path(&dll) }
            .map_err(|e| format!("failed to load wintun.dll: {e}"))?;

        let adapter_name = "MNX-LAN";
        let adapter = match wintun::Adapter::open(&wintun, adapter_name) {
            Ok(a) => a,
            Err(_) => wintun::Adapter::create(&wintun, adapter_name, "MNX LAN", None)
                .map_err(|e| format!("failed to create Wintun adapter: {e}"))?,
        };
        let session = Arc::new(
            adapter
                .start_session(wintun::MAX_RING_CAPACITY)
                .map_err(|e| format!("failed to start session: {e}"))?,
        );

        let tun = TunAdapter {
            session: session.clone(),
            _adapter: adapter,
        };

        let _ = std::process::Command::new("netsh")
            .args(["interface", "set", "interface", "MNX-LAN", "admin=enabled"])
            .output();
        Ok(tun)
    }

    pub fn session(&self) -> Arc<wintun::Session> {
        self.session.clone()
    }

    pub fn set_ip(&self, ip: &str) {
        let _ = std::process::Command::new("netsh")
            .args([
                "interface",
                "ip",
                "set",
                "address",
                "name=MNX-LAN",
                "source=static",
                ip,
                VIRTUAL_MASK,
                "gateway=none",
            ])
            .output();
    }

    pub fn inject(&self, ip_packet: &[u8]) -> Result<(), String> {
        let len = u16::try_from(ip_packet.len()).map_err(|_| "packet too large")?;
        let mut pkt = self
            .session
            .allocate_send_packet(len)
            .map_err(|e| e.to_string())?;
        pkt.bytes_mut().copy_from_slice(ip_packet);
        self.session.send_packet(pkt);
        Ok(())
    }
}

pub fn spawn_tun_rx_loop(
    session: Arc<wintun::Session>,
    tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
) {
    std::thread::spawn(move || {
        loop {
            match session.receive_blocking() {
                Ok(pkt) => {
                    if tx.is_closed() {
                        break;
                    }
                    let bytes = pkt.bytes().to_vec();
                    if tx.send(bytes).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
}