use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use serde_json::json;
use tauri::Emitter;
use tokio::sync::{mpsc, oneshot};

use crate::lobby::protocol::{
    decode_frame, encode_frame, fresh_id, now_ms, v4_to_u32, F_BROADCAST, F_CONTROL, F_TUNNEL,
    FRAME_HEADER, HOST_VIP, LobbyMsg, PeerInfo, SIGNAL_PORT,
};
use crate::lobby::room::Room;
use crate::network::adapter::{self, Tun};
use crate::network::broadcast::{is_broadcast_candidate, packet_src_dst, rebuild_broadcast};
use crate::network::peer::{new_udp_socket, PeerRuntime};
use crate::network::stun::discover_public_addr;

pub struct Engine {
    rt: Arc<tokio::runtime::Runtime>,
    dll_candidates: Vec<PathBuf>,
    self_id: String,
    self_name: Mutex<String>,
    tasks_started: AtomicBool,
    weak_self: Mutex<Option<Weak<Self>>>,
    inner: Mutex<Inner>,
    handle: Mutex<Option<tauri::AppHandle>>,
}

struct Inner {
    room: Option<Room>,
    member_code: Option<String>,
    peers: HashMap<String, PeerRuntime>,
    socket: Option<Arc<tokio::net::UdpSocket>>,
    tun: Option<Tun>,
    packet_rx: Option<mpsc::Receiver<Vec<u8>>>,
    public: Option<SocketAddr>,
    my_vip: u32,
    pending_vip: Option<oneshot::Sender<String>>,
}

impl Engine {
    pub fn new(rt: tokio::runtime::Runtime, dll_candidates: Vec<PathBuf>) -> Arc<Self> {
        let engine = Arc::new(Engine {
            rt: Arc::new(rt),
            dll_candidates,
            self_id: fresh_id(),
            self_name: Mutex::new(String::from("Player")),
            tasks_started: AtomicBool::new(false),
            weak_self: Mutex::new(None),
            inner: Mutex::new(Inner {
                room: None,
                member_code: None,
                peers: HashMap::new(),
                socket: None,
                tun: None,
                packet_rx: None,
                public: None,
                my_vip: 0,
                pending_vip: None,
            }),
            handle: Mutex::new(None),
        });
        *engine.weak_self.lock().unwrap() = Some(Arc::downgrade(&engine));
        engine
    }

    fn arc(&self) -> Arc<Self> {
        self.weak_self
            .lock()
            .unwrap()
            .clone()
            .and_then(|w| w.upgrade())
            .expect("engine must stay alive")
    }

    pub fn attach(&self, handle: tauri::AppHandle) {
        *self.handle.lock().unwrap() = Some(handle);
    }

    pub fn set_name(&self, name: &str) {
        *self.self_name.lock().unwrap() = name.to_string();
    }

    fn app(&self) -> Option<tauri::AppHandle> {
        self.handle.lock().unwrap().clone()
    }

    fn notify(&self, level: &str, message: &str) {
        if let Some(app) = self.app() {
            let _ = app.emit("notify", json!({ "level": level, "message": message }));
        }
    }

    fn emit_session(&self) {
        if let Some(app) = self.app() {
            let _ = app.emit("session", self.session_json());
        }
    }

    fn emit_peers(&self) {
        if let Some(app) = self.app() {
            let _ = app.emit("peers", self.peers_json());
        }
    }

    fn is_host(&self) -> bool {
        self.inner.lock().unwrap().room.is_some()
    }

    async fn bind_socket(&self) -> Result<Arc<tokio::net::UdpSocket>, String> {
        if let Some(s) = self.inner.lock().unwrap().socket.clone() {
            return Ok(s);
        }
        let bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), SIGNAL_PORT);
        let sock = new_udp_socket(bind)
            .or_else(|_| new_udp_socket(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)))?;
        let sock = Arc::new(sock);
        *self.inner.lock().unwrap().socket = Some(Arc::clone(&sock));
        Ok(sock)
    }

    async fn ensure_tun(&self, vip: u32) -> Result<(), String> {
        if self.inner.lock().unwrap().tun.is_some() {
            return Ok(());
        }
        let (dll, _path) = adapter::load_wintun(&self.dll_candidates)?;
        let adapter = adapter::ensure_adapter(&dll.0)?;
        adapter::configure(&adapter, Ipv4Addr::from(vip))?;
        let session = adapter::start_session(&adapter)?;
        let (tx, rx) = mpsc::channel::<Vec<u8>>(768);
        let running = Arc::new(AtomicBool::new(true));
        let reader = adapter::spawn_read_loop(Arc::clone(&session), tx, Arc::clone(&running));
        let mut g = self.inner.lock().unwrap();
        g.tun = Some(Tun {
            adapter,
            session,
            dll,
            running,
            reader: Some(reader),
        });
        g.packet_rx = Some(rx);
        Ok(())
    }

    fn spawn_runtime_tasks(&self) {
        if self.tasks_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let t1 = self.arc();
        self.rt.spawn(async move { t1.recv_loop().await });
        let t2 = self.arc();
        self.rt.spawn(async move { t2.tun_pump().await });
        let t3 = self.arc();
        self.rt.spawn(async move { t3.ping_task().await });
    }

    pub fn create_room(&self, name: String) -> Result<serde_json::Value, String> {
        self.rt.block_on(self.create_room_async(name))
    }

    async fn create_room_async(&self, name: String) -> Result<serde_json::Value, String> {
        let name = if name.trim().is_empty() {
            "Player".to_string()
        } else {
            name.trim().to_string()
        };
        {
            let g = self.inner.lock().unwrap();
            if g.room.is_some() || g.member_code.is_some() {
                return Err("You are already in a room. Leave it first.".to_string());
            }
        }
        self.set_name(&name);

        self.ensure_tun(HOST_VIP).await?;
        let socket = self.bind_socket().await?;
        let public = discover_public_addr(&socket).await;
        let public_str = public.map(|p| p.to_string()).unwrap_or_else(|| "0.0.0.0:0".to_string());

        {
            let mut g = self.inner.lock().unwrap();
            g.public = public;
            g.my_vip = HOST_VIP;
            let room = Room::new(name, public_str, Ipv4Addr::from(HOST_VIP).to_string());
            g.room = Some(room);
        }

        self.spawn_runtime_tasks();
        self.emit_session();
        self.emit_peers();
        self.notify("success", "Virtual adapter is up. Share your room code with your party.");
        Ok(self.session_json())
    }

    pub fn join_room(&self, target: String) -> Result<serde_json::Value, String> {
        self.rt.block_on(self.join_room_async(target))
    }

    async fn join_room_async(&self, raw: String) -> Result<serde_json::Value, String> {
        {
            let g = self.inner.lock().unwrap();
            if g.room.is_some() || g.member_code.is_some() {
                return Err("You are already in a room. Leave it first.".to_string());
            }
        }
        let (code, endpoint) = parse_connect_target(&raw)?;

        let socket = self.bind_socket().await?;
        if let Some(pub_addr) = discover_public_addr(&socket).await {
            self.inner.lock().unwrap().public = Some(pub_addr);
        }

        let name = self.self_name.lock().unwrap().clone();
        let nonce = now_ms();
        let (tx, rx) = oneshot::channel::<String>();
        {
            let mut g = self.inner.lock().unwrap();
            g.pending_vip = Some(tx);
        }

        let hello = LobbyMsg::Hello {
            room: code.clone(),
            name,
            nonce,
        };
        if let Some(ep) = endpoint {
            self.send_control(ep, &hello).await;
        } else {
            let bcast = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255)), SIGNAL_PORT);
            if let Some(sock) = self.inner.lock().unwrap().socket.clone() {
                let payload = serde_json::to_vec(&hello).unwrap_or_default();
                self.send_raw(sock, bcast, F_CONTROL, 0, 0, &payload).await;
            }
        }

        self.spawn_runtime_tasks();

        let vip = tokio::time::timeout(std::time::Duration::from_secs(6), rx)
            .await
            .ok()
            .flatten()
            .ok_or_else(|| {
                "No response from host. Verify the room code and that the host is online.".to_string()
            })?;

        {
            let mut g = self.inner.lock().unwrap();
            g.member_code = Some(code.clone());
            g.my_vip = parse_ip_u32(&vip).unwrap_or(0);
            g.pending_vip = None;
        }
        self.emit_session();
        self.emit_peers();
        self.notify("success", &format!("Connected to {code} as {vip}"));
        Ok(self.session_json())
    }

    pub fn leave_room(&self) {
        self.rt.block_on(async {
            let bye = LobbyMsg::Bye {
                id: self.self_id.clone(),
            };
            self.broadcast_control(&bye).await;
        });
        let mut g = self.inner.lock().unwrap();
        g.room = None;
        g.member_code = None;
        g.peers.clear();
        g.my_vip = 0;
        g.pending_vip = None;
        drop(g);
        self.emit_session();
        self.emit_peers();
        self.notify("info", "Left the room.");
    }

    pub fn session_json(&self) -> serde_json::Value {
        let g = self.inner.lock().unwrap();
        let (mode, code, vip) = if let Some(room) = &g.room {
            ("host", room.code.clone(), room.self_peer.vip.clone())
        } else if g.member_code.is_some() {
            (
                "member",
                g.member_code.clone().unwrap_or_default(),
                Ipv4Addr::from(g.my_vip).to_string(),
            )
        } else {
            ("idle", String::new(), Ipv4Addr::from(g.my_vip).to_string())
        };
        let public = g.public.map(|p| p.to_string()).unwrap_or_default();
        let adapter = g.tun.is_some();
        json!({
            "mode": mode,
            "room": code,
            "vip": vip,
            "public": public,
            "self_id": self.self_id,
            "self_name": self.self_name.lock().unwrap().clone(),
            "adapter": adapter,
            "port": SIGNAL_PORT
        })
    }

    pub fn peers_json(&self) -> serde_json::Value {
        let now = now_ms();
        let peers: Vec<PeerInfo> = {
            let g = self.inner.lock().unwrap();
            g.peers
                .values()
                .filter(|p| p.info.id != self.self_id)
                .map(|p| {
                    let mut pi = p.info.clone();
                    pi.p2p = p.established && now.saturating_sub(p.last_seen) < 8000;
                    pi
                })
                .collect()
        };
        json!({ "peers": peers })
    }

    async fn recv_loop(self: &Arc<Self>) {
        let mut buf = vec![0u8; 4096];
        loop {
            let socket = self.inner.lock().unwrap().socket.clone();
            let Some(socket) = socket else { break };
            let Ok((n, src)) = socket.recv_from(&mut buf).await else { continue };
            let Some((flags, s, d, payload)) = decode_frame(&buf[..n]) else { continue };
            if flags & F_CONTROL != 0 {
                self.handle_control(payload, src).await;
            }
            if flags & F_TUNNEL != 0 {
                self.handle_tunnel(flags, s, d, payload, src).await;
            }
        }
    }

    async fn handle_control(&self, payload: &[u8], src: SocketAddr) {
        let Ok(msg) = serde_json::from_slice::<LobbyMsg>(payload) else {
            return;
        };
        match msg {
            LobbyMsg::Ping { tok, ts } => {
                self.send_control(
                    src,
                    &LobbyMsg::Pong {
                        tok,
                        ts,
                        id: self.self_id.clone(),
                    },
                )
                .await;
            }
            LobbyMsg::Pong { ts, id, .. } => {
                let now = now_ms();
                let rtt = now.saturating_sub(ts);
                {
                    let mut g = self.inner.lock().unwrap();
                    let active = g.room.is_some() || g.member_code.is_some();
                    if !active {
                        return;
                    }
                    if let Some(p) = g.peers.get_mut(&id) {
                        p.info.rtt_ms = rtt;
                        p.established = true;
                        p.endpoint = Some(src);
                        p.last_seen = now;
                        p.info.p2p = true;
                    } else {
                        let info = PeerInfo {
                            id: id.clone(),
                            name: "Peer".to_string(),
                            vip: "10.88.0.x".to_string(),
                            public: src.to_string(),
                            rtt_ms: rtt,
                            p2p: true,
                        };
                        g.peers.insert(
                            id.clone(),
                            PeerRuntime {
                                info,
                                endpoint: Some(src),
                                last_seen: now,
                                established: true,
                            },
                        );
                    }
                    let alloc = g.room.as_mut().and_then(|r| r.alloc_vip());
                    if let Some(alloc) = alloc {
                        if let Some(p) = g.peers.get_mut(&id) {
                            if p.info.vip == "10.88.0.x" {
                                p.info.vip = alloc;
                            }
                        }
                    }
                }
                self.emit_peers();
            }
            LobbyMsg::Hello { room, name, .. } => {
                if self.is_host() {
                    self.handle_host_hello(room, name, src).await;
                }
            }
            LobbyMsg::Welcome { room, vip, host_name } => {
                let mut g = self.inner.lock().unwrap();
                if let Some(code) = g.member_code.as_ref() {
                    if *code == room {
                        if let Some(tx) = g.pending_vip.take() {
                            let _ = tx.send(vip.clone());
                        }
                        g.my_vip = parse_ip_u32(&vip).unwrap_or(0);
                    }
                }
                drop(g);
                if !host_name.is_empty() {
                    self.notify("success", &format!("Host: {host_name} · {room}"));
                }
            }
            LobbyMsg::PeerList { peers } => {
                for info in peers {
                    self.upsert_peer(info.clone());
                    self.punch_peer(&info.id).await;
                }
                self.emit_peers();
            }
            LobbyMsg::PeerJoin { peer } => {
                self.upsert_peer(peer.clone());
                self.punch_peer(&peer.id).await;
                self.notify("success", &format!("{} joined the room", peer.name));
                self.emit_peers();
            }
            LobbyMsg::PeerLeave { id } => {
                let name = self
                    .inner
                    .lock()
                    .unwrap()
                    .peers
                    .get(&id)
                    .map(|p| p.info.name.clone())
                    .unwrap_or_else(|| "Peer".to_string());
                self.remove_peer(&id);
                self.notify("info", &format!("{name} left the room"));
            }
            LobbyMsg::PeerPoke { peer } => {
                let endpoint = peer.public.parse::<SocketAddr>().ok();
                self.upsert_peer(peer.clone());
                if let Some(ep) = endpoint {
                    self.send_control(ep, &LobbyMsg::Punch { from: self.self_id.clone() })
                        .await;
                } else if let Some(ep) = self.peer_endpoint(&peer.id) {
                    self.send_control(ep, &LobbyMsg::Punch { from: self.self_id.clone() })
                        .await;
                }
            }
            LobbyMsg::Punch { from } => {
                if let Some(p) = self.inner.lock().unwrap().peers.get_mut(&from) {
                    p.endpoint = Some(src);
                    p.last_seen = now_ms();
                }
                let active = self.is_host() || self.inner.lock().unwrap().member_code.is_some();
                if active {
                    self.send_control(src, &LobbyMsg::Punch { from: self.self_id.clone() })
                        .await;
                }
            }
            LobbyMsg::Bye { id } => {
                self.remove_peer(&id);
            }
        }
    }

    async fn handle_host_hello(&self, code: String, name: String, src: SocketAddr) {
        let plan = {
            let mut g = self.inner.lock().unwrap();
            let Some(room) = g.room.as_mut() else {
                return;
            };
            if room.code != code {
                return;
            }
            let Some(vip) = room.alloc_vip() else {
                drop(g);
                self.notify("error", "Room is full.");
                return;
            };
            let id = fresh_id();
            let info = PeerInfo {
                id: id.clone(),
                name,
                vip: vip.clone(),
                public: src.to_string(),
                rtt_ms: 0,
                p2p: false,
            };
            let existing: Vec<PeerInfo> = g.peers.values().map(|p| p.info.clone()).collect();
            g.peers.insert(
                id.clone(),
                PeerRuntime {
                    info: info.clone(),
                    endpoint: Some(src),
                    last_seen: now_ms(),
                    established: true,
                },
            );
            room.peers.insert(id.clone(), info.clone());
            let host_name = room.host_name.clone();
            let room_code = room.code.clone();
            let socket = g.socket.clone().expect("socket must exist");
            (room_code, vip, host_name, existing, info, socket)
        };

        let socket = Arc::clone(&plan.5);
        let payload = serde_json::to_vec(&LobbyMsg::Welcome {
            room: plan.0.clone(),
            vip: plan.1.clone(),
            host_name: plan.2.clone(),
        })
        .unwrap_or_default();
        self.send_raw(socket, src, F_CONTROL, HOST_VIP, 0, &payload).await;

        let list = serde_json::to_vec(&LobbyMsg::PeerList { peers: plan.3.clone() }).unwrap_or_default();
        self.send_raw(Arc::clone(&plan.5), src, F_CONTROL, HOST_VIP, 0, &list)
            .await;

        for other in &plan.3 {
            let Some(oaddr) = self.peer_endpoint(&other.id) else {
                continue;
            };
            let join =
                serde_json::to_vec(&LobbyMsg::PeerJoin { peer: plan.4.clone() }).unwrap_or_default();
            self.send_raw(Arc::clone(&plan.5), oaddr, F_CONTROL, HOST_VIP, 0, &join)
                .await;
            let poke_new =
                serde_json::to_vec(&LobbyMsg::PeerPoke { peer: plan.4.clone() }).unwrap_or_default();
            self.send_raw(Arc::clone(&plan.5), oaddr, F_CONTROL, HOST_VIP, 0, &poke_new)
                .await;
            let poke_back =
                serde_json::to_vec(&LobbyMsg::PeerPoke { peer: other.clone() }).unwrap_or_default();
            self.send_raw(Arc::clone(&plan.5), src, F_CONTROL, HOST_VIP, 0, &poke_back)
                .await;
        }

        self.emit_peers();
        self.notify("success", &format!("{} joined as {}", plan.4.name, plan.1));
    }

    async fn handle_tunnel(&self, flags: u8, _src: u32, dst: u32, payload: &[u8], from: SocketAddr) {
        {
            let mut g = self.inner.lock().unwrap();
            for p in g.peers.values_mut() {
                if p.endpoint == Some(from) || p.info.public == from.to_string() {
                    p.last_seen = now_ms();
                    p.established = true;
                }
            }
        }
        if flags & F_BROADCAST != 0 {
            let mut out = Vec::with_capacity(payload.len() + 8);
            rebuild_broadcast(payload, &mut out);
            self.tun_write(&out);
            return;
        }
        let myvip = self.inner.lock().unwrap().my_vip;
        if myvip != 0 && dst == myvip {
            self.tun_write(payload);
        }
    }

    async fn tun_pump(self: &Arc<Self>) {
        let mut rx = self.inner.lock().unwrap().packet_rx.take();
        let Some(mut rx) = rx else { return };
        while let Some(pkt) = rx.recv().await {
            self.route_tun_packet(&pkt).await;
        }
    }

    async fn route_tun_packet(&self, pkt: &[u8]) {
        if is_broadcast_candidate(pkt).is_some() {
            self.broadcast_tunnel(pkt).await;
            return;
        }
        let Some((_src_ip, dst_ip)) = packet_src_dst(pkt) else {
            return;
        };
        let dst = Ipv4Addr::from(dst_ip);
        if dst.is_multicast() || dst == Ipv4Addr::BROADCAST || dst.is_broadcast() {
            return;
        }
        if dst_ip == HOST_VIP {
            return;
        }
        if let Some(endpoint) = self.peer_endpoint_by_vip(dst_ip) {
            let socket = self.inner.lock().unwrap().socket.clone();
            let myvip = self.inner.lock().unwrap().my_vip;
            if let Some(socket) = socket {
                self.send_raw(socket, endpoint, F_TUNNEL, myvip, 0, pkt).await;
            }
        }
    }

    async fn broadcast_tunnel(&self, pkt: &[u8]) {
        let targets: Vec<SocketAddr> = {
            let g = self.inner.lock().unwrap();
            g.peers.values().filter_map(|p| p.endpoint).collect()
        };
        let socket = self.inner.lock().unwrap().socket.clone();
        let myvip = self.inner.lock().unwrap().my_vip;
        if let Some(socket) = socket {
            for t in targets {
                self.send_raw(Arc::clone(&socket), t, F_TUNNEL | F_BROADCAST, myvip, 0, pkt)
                    .await;
            }
        }
    }

    async fn ping_task(self: &Arc<Self>) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        interval.tick().await;
        loop {
            interval.tick().await;
            let targets: Vec<SocketAddr> = {
                let g = self.inner.lock().unwrap();
                g.peers.values().filter_map(|p| p.endpoint).collect()
            };
            for t in targets {
                let ts = now_ms();
                self.send_control(t, &LobbyMsg::Ping { tok: ts, ts }).await;
            }
            self.emit_peers();
        }
    }

    async fn broadcast_control(&self, msg: &LobbyMsg) {
        let targets: Vec<SocketAddr> = {
            let g = self.inner.lock().unwrap();
            g.peers.values().filter_map(|p| p.endpoint).collect()
        };
        for t in targets {
            self.send_control(t, msg).await;
        }
    }

    fn upsert_peer(&self, info: PeerInfo) {
        let now = now_ms();
        let mut info = info;
        if info.vip.starts_with("10.88.0.x") {
            let alloc = self
                .inner
                .lock()
                .unwrap()
                .room
                .as_mut()
                .and_then(|r| r.alloc_vip());
            if let Some(v) = alloc {
                info.vip = v;
            }
        }
        let mut g = self.inner.lock().unwrap();
        if let Some(p) = g.peers.get_mut(&info.id) {
            p.info = info;
            p.last_seen = now;
        } else {
            let endpoint = info.public.parse::<SocketAddr>().ok();
            g.peers.insert(
                info.id.clone(),
                PeerRuntime {
                    info,
                    endpoint,
                    last_seen: now,
                    established: false,
                },
            );
        }
    }

    async fn punch_peer(&self, id: &str) {
        if let Some(endpoint) = self.peer_endpoint(id) {
            let msg = LobbyMsg::Punch {
                from: self.self_id.clone(),
            };
            self.send_control(endpoint, &msg).await;
        }
    }

    fn remove_peer(&self, id: &str) {
        let mut g = self.inner.lock().unwrap();
        if let Some(p) = g.peers.remove(id) {
            let name = p.info.name.clone();
            if let Some(room) = g.room.as_mut() {
                room.peers.remove(id);
            }
            drop(g);
            self.notify("info", &format!("{name} left the room"));
        }
        self.emit_peers();
    }

    fn peer_endpoint(&self, id: &str) -> Option<SocketAddr> {
        self.inner.lock().unwrap().peers.get(id).and_then(|p| p.endpoint)
    }

    fn peer_endpoint_by_vip(&self, vip: u32) -> Option<SocketAddr> {
        let g = self.inner.lock().unwrap();
        g.peers.values().find_map(|p| {
            let Ok(addr) = p.info.vip.parse::<Ipv4Addr>() else {
                return None;
            };
            if v4_to_u32(addr) == vip {
                p.endpoint
            } else {
                None
            }
        })
    }

    fn tun_write(&self, bytes: &[u8]) {
        let g = self.inner.lock().unwrap();
        if let Some(tun) = &g.tun {
            let _ = adapter::write_ip(&tun.session, bytes);
        }
    }

    async fn send_raw(
        &self,
        socket: Arc<tokio::net::UdpSocket>,
        addr: SocketAddr,
        flags: u8,
        src: u32,
        dst: u32,
        payload: &[u8],
    ) {
        let mut frame = Vec::with_capacity(FRAME_HEADER + payload.len());
        encode_frame(flags, src, dst, payload, &mut frame);
        let _ = socket.send_to(&frame, addr).await;
    }

    async fn send_control(&self, addr: SocketAddr, msg: &LobbyMsg) {
        let Ok(payload) = serde_json::to_vec(msg) else {
            return;
        };
        let socket = self.inner.lock().unwrap().socket.clone();
        let myvip = self.inner.lock().unwrap().my_vip;
        if let Some(socket) = socket {
            self.send_raw(socket, addr, F_CONTROL, myvip, 0, &payload).await;
        }
    }
}

fn parse_ip_u32(s: &str) -> Option<u32> {
    s.parse::<Ipv4Addr>().ok().map(v4_to_u32)
}

fn parse_connect_target(raw: &str) -> Result<(String, Option<SocketAddr>), String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Enter a room code (MNX-XXXX) or connect string (MNX-XXXX@host:port).".to_string());
    }
    if let Some(idx) = raw.find('@') {
        let code = raw[..idx].trim().to_uppercase();
        let addr_part = raw[idx + 1..].trim();
        let addr = if addr_part.contains(':') {
            addr_part.to_string()
        } else {
            format!("{addr_part}:{SIGNAL_PORT}")
        };
        let parsed = addr
            .parse::<SocketAddr>()
            .map_err(|_| format!("Invalid host endpoint '{addr_part}'. Expected ip:port."))?;
        Ok((code, Some(parsed)))
    } else {
        let code = raw.to_uppercase();
        if !code.starts_with("MNX-") {
            return Err(format!("Invalid room code '{code}'. Expected MNX-XXXX."));
        }
        Ok((code, None))
    }
}