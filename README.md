# MNX LAN

**Zero-latency Virtual LAN for LAN gaming — by RADIN MNX.**

Ultra-lightweight (<12 MB) gaming suite: a real **Wintun kernel driver** TUN adapter routes
packets through **direct peer-to-peer UDP tunnels** with **STUN/NAT hole-punching** and
**zero relay servers**. Wrapped in a native **Windows 11 Fluent / Mica** UI.

## Feature set

- **Kernel virtual adapter** — creates a `10.88.0.0/16` Wintun interface (host = `10.88.0.1`).
- **Direct P2P NAT traversal** — STUN RFC 5389 binding + simultaneous UDP hole-punching.
- **LAN broadcast replicator** — forwards game discovery UDP broadcasts (`27015`, `19132`,
  `6112`, `7777`, `25565`, …) on `255.255.255.255` to every peer.
- **Priority sockets** — DSCP `EF` (46) / IP_TOS set on every UDP socket.
- **Live peer matrix** — virtual IP one-click copy, direct-P2P status, real-time ping badges.
- **Fluent 2 UI** — native Mica backdrop, 12 px rounded corners, dark/light reactive.

## Connect

| Mode | What to share |
| --- | --- |
| Same LAN / party | Room code `MNX-XXXX` — auto-discovered via UDP broadcast on port `38888` |
| Across the internet | Connect string `MNX-XXXX@<host-public-ip>:38888` |

## Tech stack

- Rust: `tokio` · `wintun` · `stun` · `socket2` · `serde` — Tauri v2 backend
- Frontend: vanilla Fluent 2 (Mica) — `index.html` · `app.js` · `fluent-mica.css`
- CI: GitHub Actions `windows-latest` → NSIS installer + GitHub Release

```
MNX-LAN/
├── src-tauri/
│   ├── Cargo.toml            # tokio, wintun, stun, tauri v2, serde
│   ├── tauri.conf.json       # Borderless, Mica effect, admin manifest
│   ├── assets/wintun.dll     # x64 driver (Wintun 0.14.1)
│   └── src/
│       ├── main.rs           # Engine entrypoint & IPC router
│       ├── network/
│       │   ├── adapter.rs    # Wintun TUN creation & IP assignment
│       │   ├── peer.rs       # P2P UDP sockets (DSCP QoS)
│       │   ├── broadcast.rs  # LAN game broadcast replicator
│       │   └── stun.rs       # NAT discovery (RFC 5389)
│       └── lobby/
│           ├── room.rs       # Room state & virtual-IP allocator
│           └── protocol.rs   # Framing & room serialization
└── src/                      # Fluent / Mica UI
    ├── index.html
    ├── app.js
    └── styles/fluent-mica.css
```

## Build

```powershell
npm install
npm run tauri build
```

Run as **Administrator** — creating a Wintun adapter requires elevation (embedded in the
application manifest).