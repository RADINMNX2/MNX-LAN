#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod lobby;
mod network;
mod selftest;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use lobby::protocol::*;
use lobby::room::{Room, HOST_VIP};
use network::adapter::TunAdapter;
use network::broadcast::BroadcastReplicator;
use network::peer::Peer;
use serde::Serialize;
use tauri::{Emitter, Manager, State};

const HEARTBEAT_EVICT_MS: u64 = 30_000;

#[derive(Serialize, Clone)]
pub struct PeerView {
    pub name: String,
    pub virtual_ip: String,
    pub local_addr: String,
    pub mapped_addr: String,
    pub nat_ok: bool,
    pub rtt_ms: Option<u64>,
    pub last_seen_ms: u64,
}

#[derive(Serialize, Clone)]
pub struct RoomView {
    pub id: String,
    pub host: String,
    pub my_vip: String,
    pub peer_count: usize,
}

#[derive(Serialize, Clone)]
pub struct CreateRoomResult {
    pub room_id: String,
    pub my_vip: String,
    pub mapped_addr: String,
    pub nat_ok: bool,
}

struct EngineInner {
    name: String,
    is_host: RwLock<bool>,
    room: RwLock<Option<Room>>,
    my_vip: RwLock<String>,
    peers: Mutex<HashMap<String, Peer>>,
    socket: Mutex<Option<Arc<tokio::net::UdpSocket>>>,
    host_endpoint: RwLock<Option<SocketAddr>>,
    mapped_addr: RwLock<Option<SocketAddr>>,
    nat_ok: RwLock<bool>,
    tun: Mutex<Option<Arc<TunAdapter>>>,
    tun_tx: Mutex<Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>>,
    replicator: Mutex<BroadcastReplicator>,
    ack_tx: tokio::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<CtrlMsg>>>,
    app: Mutex<Option<tauri::AppHandle>>,
}

#[derive(Clone)]
pub struct Engine {
    inner: Arc<EngineInner>,
}

impl Engine {
    fn new(name: String, app: tauri::AppHandle) -> Self {
        Engine {
            inner: Arc::new(EngineInner {
                name,
                is_host: RwLock::new(false),
                room: RwLock::new(None),
                my_vip: RwLock::new(HOST_VIP.to_string()),
                peers: Mutex::new(HashMap::new()),
                socket: Mutex::new(None),
                host_endpoint: RwLock::new(None),
                mapped_addr: RwLock::new(None),
                nat_ok: RwLock::new(false),
                tun: Mutex::new(None),
                tun_tx: Mutex::new(None),
                replicator: Mutex::new(BroadcastReplicator::new(&[])),
                ack_tx: tokio::sync::Mutex::new(None),
                app: Mutex::new(Some(app)),
            }),
        }
    }

    fn emit_event(&self, event: &str, payload: impl Serialize + Clone) {
        if let Some(app) = self.inner.app.lock().unwrap().clone() {
            let _ = app.emit(event, payload);
        }
    }

    fn socket(&self) -> Option<Arc<tokio::net::UdpSocket>> {
        self.inner.socket.lock().unwrap().clone()
    }

    fn try_setup_tun(&self) -> Result<Arc<TunAdapter>, String> {
        let mut guard = self.inner.tun.lock().unwrap();
        if let Some(existing) = guard.as_ref() {
            return Ok(existing.clone());
        }
        let tun = Arc::new(TunAdapter::create()?);
        tun.set_ip(&self.inner.my_vip.read().unwrap().clone());
        *guard = Some(tun.clone());
        Ok(tun)
    }

    fn start_engine_tasks(&self) {
        self.spawn_control_loop();
        self.spawn_tun_relay();
        self.spawn_ping_loop();
        self.spawn_gc_loop();
    }

    async fn create_room(&self) -> Result<CreateRoomResult, String> {
        if self.inner.room.read().unwrap().is_some() {
            return Err("already in a room".into());
        }
        if !network::adapter::is_elevated() {
            return Err("admin".into());
        }

        let tun = self.try_setup_tun()?;
        tun.set_ip(HOST_VIP);
        *self.inner.my_vip.write().unwrap() = HOST_VIP.to_string();

        let sock = network::peer::bind_tos_socket()?;
        let sock = Arc::new(sock);
        *self.inner.socket.lock().unwrap() = Some(sock.clone());

        let mapped = network::stun::mapped_addr(&sock).await;
        let (mapped_str, nat_ok) = match mapped {
            Ok(a) => {
                *self.inner.mapped_addr.write().unwrap() = Some(a);
                *self.inner.nat_ok.write().unwrap() = true;
                (a.to_string(), true)
            }
            Err(e) => {
                *self.inner.nat_ok.write().unwrap() = false;
                self.emit_event("nat-status", format!("NAT probe failed: {e}"));
                (String::new(), false)
            }
        };

        let room = Room::create(&self.inner.name);
        let room_id = room.id.clone();
        *self.inner.room.write().unwrap() = Some(room);
        *self.inner.is_host.write().unwrap() = true;
        *self.inner.host_endpoint.write().unwrap() = sock.local_addr().ok();

        self.start_engine_tasks();

        let result = CreateRoomResult {
            room_id,
            my_vip: HOST_VIP.to_string(),
            mapped_addr: mapped_str,
            nat_ok,
        };
        self.emit_event("room-ready", result.clone());
        Ok(result)
    }

    async fn join_room(
        &self,
        room_id: &str,
        host: &str,
        port: u16,
    ) -> Result<CreateRoomResult, String> {
        if self.inner.room.read().unwrap().is_some() {
            return Err("already in a room".into());
        }
        if !network::adapter::is_elevated() {
            return Err("admin".into());
        }

        let tun = self.try_setup_tun()?;

        let sock = network::peer::bind_tos_socket()?;
        let sock = Arc::new(sock);
        *self.inner.socket.lock().unwrap() = Some(sock.clone());

        let mapped = network::stun::mapped_addr(&sock).await;
        let (mapped_str, nat_ok) = match mapped {
            Ok(a) => {
                *self.inner.mapped_addr.write().unwrap() = Some(a);
                *self.inner.nat_ok.write().unwrap() = true;
                (a.to_string(), true)
            }
            Err(e) => {
                *self.inner.nat_ok.write().unwrap() = false;
                self.emit_event("nat-status", format!("NAT probe failed: {e}"));
                (String::new(), false)
            }
        };

        let host_ep: SocketAddr = format!("{host}:{port}").parse().map_err(|e| format!("bad host: {e}"))?;
        *self.inner.host_endpoint.write().unwrap() = Some(host_ep);

        let (ack_tx, mut ack_rx) = tokio::sync::mpsc::unbounded_channel();
        *self.inner.ack_tx.lock().await = Some(ack_tx);

        self.start_engine_tasks();

        let req = CtrlMsg::JoinRequest {
            room: room_id.to_string(),
            name: self.inner.name.clone(),
            mapped_addr: mapped_str.clone(),
        };
        let frame = ctrl_frame(&req)?;
        sock.send_to(&frame, host_ep).await.map_err(|e| e.to_string())?;

        let ack = tokio::time::timeout(Duration::from_secs(8), ack_rx.recv())
            .await
            .map_err(|_| "join timed out - is the host online?")?
            .ok_or_else(|| String::from("join channel closed"))?;

        match ack {
            CtrlMsg::JoinAck { room, host, vip, peers } => {
                let room_state = Room {
                    id: room.clone(),
                    host_name: host.clone(),
                    peers: HashMap::new(),
                    next_vip: 2,
                };
                *self.inner.room.write().unwrap() = Some(room_state);
                *self.inner.my_vip.write().unwrap() = vip.clone();
                tun.set_ip(&vip);

                {
                    let mut map = self.inner.peers.lock().unwrap();
                    for p in &peers {
                        if p.name == self.inner.name {
                            continue;
                        }
                        if let Ok(ep) = p.local_addr.parse::<SocketAddr>() {
                            let peer = Peer::new(
                                p.name.clone(),
                                p.virtual_ip.clone(),
                                p.local_addr.clone(),
                                p.mapped_addr.clone(),
                                ep,
                            );
                            map.insert(p.name.clone(), peer);
                        }
                    }
                }

                for p in &peers {
                    if p.name == self.inner.name {
                        continue;
                    }
                    if let Ok(ep) = p.local_addr.parse::<SocketAddr>() {
                        let _ = sock.try_send_to(&ping_frame(now_ms()), ep);
                    }
                }

                self.emit_event(
                    "peer-list",
                    (room, host, vip, peers.len()).to_string(),
                );
                Ok(CreateRoomResult {
                    room_id: room,
                    my_vip: vip,
                    mapped_addr: mapped_str,
                    nat_ok,
                })
            }
            _ => Err("unexpected join response".into()),
        }
    }

    async fn leave_room(&self) -> Result<(), String> {
        let gone = {
            let room = self.inner.room.read().unwrap().clone();
            let is_host = *self.inner.is_host.read().unwrap();
            (room, is_host)
        };
        if let Some(room) = gone.0 {
            if room.peers.is_empty() {
                // nothing to notify
            }
            if let (Some(sock), Some(host_ep)) = (self.socket(), *self.inner.host_endpoint.read().unwrap()) {
                if !gone.1 {
                    let frame = ctrl_frame(&CtrlMsg::PeerGone {
                        name: self.inner.name.clone(),
                    })
                    .map_err(|e| e.to_string())?;
                    let _ = sock.try_send_to(&frame, host_ep);
                }
            }
        }
        self.shutdown_engine();
        Ok(())
    }

    fn shutdown_engine(&self) {
        *self.inner.room.write().unwrap() = None;
        *self.inner.is_host.write().unwrap() = false;
        *self.inner.host_endpoint.write().unwrap() = None;
        *self.inner.mapped_addr.write().unwrap() = None;
        *self.inner.nat_ok.write().unwrap() = false;
        self.inner.peers.lock().unwrap().clear();
        *self.inner.tun_tx.lock().unwrap() = None;
        *self.inner.tun.lock().unwrap() = None;
        *self.inner.socket.lock().unwrap() = None;
    }

    fn peers_snapshot(&self) -> Vec<PeerView> {
        let mut views = Vec::new();
        let peers = self.inner.peers.lock().unwrap();
        let now = Instant::now();
        for p in peers.values() {
            views.push(PeerView {
                name: p.name.clone(),
                virtual_ip: p.virtual_ip.clone(),
                local_addr: p.local_addr.clone(),
                mapped_addr: p.mapped_addr.clone(),
                nat_ok: !p.mapped_addr.is_empty(),
                rtt_ms: p.rtt_ms,
                last_seen_ms: now.duration_since(p.last_seen).as_millis() as u64,
            });
        }
        views.sort_by(|a, b| a.virtual_ip.cmp(&b.virtual_ip));
        views
    }

    fn room_info(&self) -> Result<RoomView, String> {
        let room = self.inner.room.read().unwrap().clone().ok_or("not in room")?;
        let peer_count = self.inner.peers.lock().unwrap().len();
        Ok(RoomView {
            id: room.id,
            host: room.host_name,
            my_vip: self.inner.my_vip.read().unwrap().clone(),
            peer_count,
        })
    }

    fn set_custom_ports(&self, ports: Vec<u16>) -> Result<(), String> {
        self.inner.replicator.lock().unwrap().set_custom_ports(&ports);
        Ok(())
    }

    fn spawn_control_loop(&self) {
        let eng = self.clone();
        tauri::async_runtime::spawn(async move {
            let Some(sock) = eng.socket() else { return };
            let mut buf = [0u8; 65535];
            loop {
                let (n, src) = match sock.recv_from(&mut buf).await {
                    Ok(v) => v,
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                };
                if n < HEADER_LEN {
                    continue;
                }
                let Some((kind, payload)) = decode_frame(&buf[..n]) else {
                    continue;
                };
                match kind {
                    FrameKind::Ctrl => {
                        let Ok(msg) = serde_json::from_slice::<CtrlMsg>(payload) else {
                            continue;
                        };
                        match msg {
                            CtrlMsg::JoinRequest { room, name, mapped_addr } => {
                                eng.on_join_request(&room, &name, &mapped_addr, src).await;
                            }
                            CtrlMsg::JoinAck { room: _, host: _, vip: _, peers: _ } => {
                                if let Some(tx) = eng.inner.ack_tx.lock().await.take() {
                                    let _ = tx.send(msg);
                                }
                            }
                            CtrlMsg::PeerJoined { peer } => {
                                eng.on_peer_joined(&peer, src);
                            }
                            CtrlMsg::PeerGone { name } => {
                                eng.on_peer_gone(&name);
                            }
                            CtrlMsg::Heartbeat { room, name } => {
                                if eng.in_room(&room) {
                                    eng.touch_peer(&name, src);
                                }
                            }
                        }
                    }
                    FrameKind::Data => {
                        eng.on_data_frame(payload.to_vec());
                    }
                    FrameKind::Ping => {
                        let ts = ts_from_payload(payload);
                        eng.touch_peer_by_src(src);
                        let _ = sock.try_send_to(&pong_frame(ts), src);
                    }
                    FrameKind::Pong => {
                        let ts = ts_from_payload(payload);
                        eng.on_pong(src, ts);
                    }
                    FrameKind::Unknown => {}
                }
            }
        });
    }

    fn in_room(&self, room_id: &str) -> bool {
        self.inner
            .room
            .read()
            .unwrap()
            .as_ref()
            .map(|r| r.id == room_id)
            .unwrap_or(false)
    }

    fn touch_peer(&self, name: &str, _src: SocketAddr) {
        let mut map = self.inner.peers.lock().unwrap();
        if let Some(p) = map.get_mut(name) {
            p.last_seen = Instant::now();
        }
    }

    fn touch_peer_by_src(&self, src: SocketAddr) {
        let mut map = self.inner.peers.lock().unwrap();
        if let Some(p) = map.values_mut().find(|p| p.endpoint == src) {
            p.last_seen = Instant::now();
        }
    }

    async fn on_join_request(&self, room_id: &str, name: &str, mapped_addr: &str, src: SocketAddr) {
        if !self.in_room(room_id) {
            return;
        }
        let mut my_room_opt = self.inner.room.write().unwrap();
        let Some(room) = my_room_opt.as_mut() else { return };
        if room.peers.contains_key(name) {
            return;
        }
        let vip = room.next_virtual_ip();
        let local_addr = src.to_string();
        room.peers.insert(
            name.to_string(),
            lobby::room::RoomPeer {
                name: name.to_string(),
                virtual_ip: vip.clone(),
                local_addr: local_addr.clone(),
                mapped_addr: mapped_addr.to_string(),
            },
        );
        let room_id = room.id.clone();
        let host_name = room.host_name.clone();
        let peers_wire: Vec<PeerInfoWire> = room
            .peers
            .iter()
            .map(|(_, p)| PeerInfoWire {
                name: p.name.clone(),
                virtual_ip: p.virtual_ip.clone(),
                local_addr: p.local_addr.clone(),
                mapped_addr: p.mapped_addr.clone(),
            })
            .collect();
        drop(my_room_opt);

        let mut map = self.inner.peers.lock().unwrap();
        let ep = src;
        map.insert(
            name.to_string(),
            Peer::new(
                name.to_string(),
                vip.clone(),
                local_addr.clone(),
                mapped_addr.to_string(),
                src,
            ),
        );
        drop(map);

        if let Some(sock) = self.socket() {
            let ack = ctrl_frame(&CtrlMsg::JoinAck {
                room: room_id,
                host: host_name,
                vip: vip.clone(),
                peers: peers_wire.clone(),
            });
            if let Ok(frame) = ack {
                let _ = sock.try_send_to(&frame, ep);
            }
            let _ = sock.try_send_to(&ping_frame(now_ms()), ep);

            for other in &peers_wire {
                if other.name == name {
                    continue;
                }
                let joined = ctrl_frame(&CtrlMsg::PeerJoined {
                    peer: PeerInfoWire {
                        name: name.to_string(),
                        virtual_ip: vip.clone(),
                        local_addr: local_addr.clone(),
                        mapped_addr: mapped_addr.to_string(),
                    },
                });
                if let Ok(frame) = joined {
                    if let Ok(ep2) = other.local_addr.parse::<SocketAddr>() {
                        let _ = sock.try_send_to(&frame, ep2);
                    }
                }
            }
        }
        self.emit_event(
            "peer-joined",
            (name.to_string(), vip, local_addr, mapped_addr.to_string()),
        );
        let _ = ep;
    }

    fn on_peer_joined(&self, peer: &PeerInfoWire, src: SocketAddr) {
        if peer.name == self.inner.name {
            return;
        }
        let mut map = self.inner.peers.lock().unwrap();
        map.insert(
            peer.name.clone(),
            Peer::new(
                peer.name.clone(),
                peer.virtual_ip.clone(),
                peer.local_addr.clone(),
                peer.mapped_addr.clone(),
                src,
            ),
        );
        drop(map);
        if let Some(sock) = self.socket() {
            let _ = sock.try_send_to(&ping_frame(now_ms()), src);
        }
        self.emit_event("peer-joined", (peer.name.clone(), peer.virtual_ip.clone()));
    }

    fn on_peer_gone(&self, name: &str) {
        {
            let mut map = self.inner.peers.lock().unwrap();
            map.remove(name);
        }
        {
            let mut room_opt = self.inner.room.write().unwrap();
            if let Some(room) = room_opt.as_mut() {
                room.peers.remove(name);
            }
        }
        self.emit_event("peer-left", name.to_string());
    }

    fn on_pong(&self, src: SocketAddr, ts: u64) {
        if ts == 0 {
            return;
        }
        let sent = Duration::from_millis(ts);
        let now = Duration::from_millis(now_ms());
        let rtt = now.saturating_sub(sent).as_millis() as u64;
        let mut map = self.inner.peers.lock().unwrap();
        if let Some(p) = map.values_mut().find(|p| p.endpoint == src) {
            p.rtt_ms = Some(rtt);
            p.last_seen = Instant::now();
            let name = p.name.clone();
            drop(map);
            self.emit_event("ping-update", (name, rtt));
        }
    }

    fn on_data_frame(&self, pkt: Vec<u8>) {
        let out = self.inner.replicator.lock().unwrap().should_replicate(&pkt);
        if let Some(out) = out {
            if let Some(tun) = self.inner.tun.lock().unwrap().clone() {
                let _ = tun.inject(&out);
            }
        }
    }

    fn spawn_tun_relay(&self) {
        let eng = self.clone();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        *self.inner.tun_tx.lock().unwrap() = Some(tx);

        tauri::async_runtime::spawn(async move {
            while let Some(pkt) = rx.recv().await {
                let out = eng.inner.replicator.lock().unwrap().should_replicate(&pkt);
                let Some(out) = out else { continue };
                let Some(sock) = eng.socket() else { continue };
                let peers = eng.inner.peers.lock().unwrap().clone();
                let frame = data_frame(&out);
                for (_, p) in &peers {
                    let _ = sock.try_send_to(&frame, p.endpoint);
                }
            }
        });
    }

    fn spawn_ping_loop(&self) {
        let eng = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let Some(sock) = eng.socket() else { continue };
                let peers = eng.inner.peers.lock().unwrap().clone();
                if peers.is_empty() {
                    continue;
                }
                let frame = ping_frame(now_ms());
                for (_, p) in &peers {
                    let _ = sock.try_send_to(&frame, p.endpoint);
                }
            }
        });
    }

    fn spawn_gc_loop(&self) {
        let eng = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                if !*eng.inner.is_host.read().unwrap() {
                    continue;
                }
                let now = Instant::now();
                let stale: Vec<String> = {
                    let map = eng.inner.peers.lock().unwrap();
                    map.iter()
                        .filter(|(_, p)| now.duration_since(p.last_seen).as_millis() as u64 > HEARTBEAT_EVICT_MS)
                        .map(|(k, _)| k.clone())
                        .collect()
                };
                for name in stale {
                    eng.on_peer_gone(&name);
                    if let (Some(sock), Some(host_ep)) = (eng.socket(), *eng.inner.host_endpoint.read().unwrap()) {
                        let _ = sock.try_send_to(&pong_frame(0), host_ep);
                    }
                }
            }
        });
    }
}

#[tauri::command]
async fn create_room(engine: State<'_, Engine>) -> Result<CreateRoomResult, String> {
    let eng = engine.inner().clone();
    eng.create_room().await
}

#[tauri::command]
async fn join_room(
    engine: State<'_, Engine>,
    room: String,
    host: String,
    port: u16,
) -> Result<CreateRoomResult, String> {
    let eng = engine.inner().clone();
    eng.join_room(&room, &host, port).await
}

#[tauri::command]
async fn leave_room(engine: State<'_, Engine>) -> Result<(), String> {
    let eng = engine.inner().clone();
    eng.leave_room().await
}

#[tauri::command]
fn peers_snapshot(engine: State<'_, Engine>) -> Result<Vec<PeerView>, String> {
    Ok(engine.inner().peers_snapshot())
}

#[tauri::command]
fn room_info(engine: State<'_, Engine>) -> Result<RoomView, String> {
    engine.inner().room_info()
}

#[tauri::command]
fn set_custom_ports(engine: State<'_, Engine>, ports: Vec<u16>) -> Result<(), String> {
    engine.inner().set_custom_ports(ports)
}

#[tauri::command]
fn is_admin() -> Result<bool, String> {
    Ok(network::adapter::is_elevated())
}

#[tauri::command]
fn relaunch_elevated(app: tauri::AppHandle) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let cmd = format!("Start-Process -FilePath '{}' -Verb RunAs", exe.to_string_lossy());
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &cmd])
        .spawn()
        .map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(1));
        app.exit(0);
    });
    Ok(())
}

fn apply_mica(app: &mut tauri::App) {
    #[cfg(target_os = "windows")]
    {
        use tauri::window::{Effect, EffectState, EffectsBuilder};
        if let Some(window) = app.get_webview_window("main") {
            let result = window.set_effects(
                EffectsBuilder::new()
                    .effect(Effect::Mica)
                    .state(EffectState::Active)
                    .radius(12.0)
                    .build(),
            );
            if result.is_err() {
                let _ = window.set_background_color(Some(tauri::window::Color(28, 28, 30, 255)));
            }
        }
    }
}

fn run_tauri() {
    tauri::Builder::default()
        .setup(|app| {
            apply_mica(app);
            let engine = Engine::new("player".to_string(), app.handle().clone());
            app.manage(engine);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_room,
            join_room,
            leave_room,
            peers_snapshot,
            room_info,
            set_custom_ports,
            is_admin,
            relaunch_elevated
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn main() {
    if std::env::args().any(|a| a == "--selftest") {
        let rt = tokio::runtime::Runtime::new().expect("failed to create selftest runtime");
        let ok = rt.block_on(selftest::run());
        println!("MNX-LAN selftest: {}", if ok { "PASS" } else { "FAIL" });
        std::process::exit(if ok { 0 } else { 1 });
    }
    run_tauri();
}