"use strict";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;

const appWindow = getCurrentWindow();

const $ = (id) => document.getElementById(id);

let peerCache = {};

function toast(text, ms = 2600) {
  const host = $("toast-host");
  const el = document.createElement("div");
  el.className = "toast";
  el.textContent = text;
  host.appendChild(el);
  setTimeout(() => {
    el.classList.add("show-remove");
    setTimeout(() => el.remove(), 320);
  }, ms);
}

async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    toast(`Copied ${text}`);
  } catch {
    toast("Copy failed");
  }
}

// ── Cursor-tracked glow ─────────────────────
["btn-create", "btn-join", "btn-leave"].forEach((id) => {
  const b = $(id);
  b.addEventListener("pointermove", (e) => {
    const r = b.getBoundingClientRect();
    b.style.setProperty("--mx", `${e.clientX - r.left}px`);
    b.style.setProperty("--my", `${e.clientY - r.top}px`);
  });
});

// ── Titlebar ────────────────────────────────
$("btn-min").onclick = () => appWindow.minimize().catch(() => {});
$("btn-max").onclick = () => appWindow.toggleMaximize().catch(() => {});
$("btn-close").onclick = () => appWindow.close().catch(() => {});

function cycleTheme() {
  const html = document.documentElement;
  const next = html.getAttribute("data-theme") === "dark" ? "light" : "dark";
  html.setAttribute("data-theme", next);
  localStorage.setItem("mnx-theme", next);
}
$("btn-theme").onclick = () => cycleTheme();
$("btn-theme").addEventListener("dblclick", (e) => e.stopPropagation());
const savedTheme = localStorage.getItem("mnx-theme");
if (savedTheme) document.documentElement.setAttribute("data-theme", savedTheme);

// ── Elevation guard ─────────────────────────
async function checkAdmin() {
  try {
    const admin = await invoke("is_admin");
    if (!admin) {
      $("admin-warning").classList.remove("hidden");
    }
  } catch { /* ignore */ }
}
$("btn-elevate").onclick = async () => {
  try {
    await invoke("relaunch_elevated");
  } catch (e) {
    toast(`Elevation failed: ${e}`);
  }
};

// ── Room flow ───────────────────────────────
function showInRoom(roomId, vip) {
  $("not-in-room").classList.add("hidden");
  $("in-room").classList.remove("hidden");
  $("room-id").textContent = roomId;
  $("room-code-copy").textContent = roomId;
  $("vip-copy").textContent = vip;
}

function showNotInRoom() {
  $("in-room").classList.add("hidden");
  $("not-in-room").classList.remove("hidden");
  $("room-id").textContent = "—";
  peerCache = {};
  renderPeerMatrix();
}

$("btn-create").onclick = async () => {
  $("btn-create").disabled = true;
  try {
    const res = await invoke("create_room");
    showInRoom(res.room_id, res.my_vip);
    $("nat-badge").textContent = res.nat_ok ? "Open" : "Strict";
    $("nat-badge").className = "badge " + (res.nat_ok ? "badge-ok" : "badge-warn");
    toast(`Room ${res.room_id} created`);
  } catch (e) {
    if (e === "admin") toast("Launch as administrator first");
    else toast(`Create failed: ${e}`);
  } finally {
    $("btn-create").disabled = false;
  }
};

$("btn-join").onclick = async () => {
  const room = $("input-room").value.trim().toUpperCase();
  const host = $("input-host").value.trim();
  const port = parseInt($("input-port").value, 10);
  if (!room || !host || !port) {
    toast("Fill in room, host and port");
    return;
  }
  $("btn-join").disabled = true;
  try {
    const res = await invoke("join_room", { room, host, port });
    showInRoom(res.room_id, res.my_vip);
    $("nat-badge").textContent = res.nat_ok ? "Open" : "Strict";
    $("nat-badge").className = "badge " + (res.nat_ok ? "badge-ok" : "badge-warn");
    toast(`Joined ${res.room_id}`);
  } catch (e) {
    if (e === "admin") toast("Launch as administrator first");
    else toast(`Join failed: ${e}`);
  } finally {
    $("btn-join").disabled = false;
  }
};

$("btn-leave").onclick = async () => {
  try {
    await invoke("leave_room");
    showNotInRoom();
    toast("Left room");
  } catch (e) {
    toast(`Leave failed: ${e}`);
  }
};

$("room-code-copy").onclick = () => copyText($("room-code-copy").textContent);
$("vip-copy").onclick = () => copyText($("vip-copy").textContent);

$("btn-apply-ports").onclick = async () => {
  const raw = $("input-ports").value.trim();
  if (!raw) return;
  const ports = raw.split(",").map((s) => parseInt(s.trim(), 10)).filter((n) => n > 0 && n < 65536);
  try {
    await invoke("set_custom_ports", { ports });
    toast(`Broadcast ports: ${ports.join(", ")}`);
  } catch (e) {
    toast(`Apply failed: ${e}`);
  }
};
$("input-ports").addEventListener("keydown", (e) => {
  if (e.key === "Enter") $("btn-apply-ports").click();
});

// ── Peer matrix ─────────────────────────────
function buildPeerCard(peer, idx) {
  const card = document.createElement("div");
  card.className = "peer-card";
  card.dataset.name = peer.name;
  const initials = peer.name.slice(0, 2).toUpperCase();
  const rtt = peer.rtt_ms == null ? "—" : `${peer.rtt_ms}ms`;
  const rttClass = peer.rtt_ms == null ? "" : peer.rtt_ms <= 40 ? " ok" : peer.rtt_ms <= 120 ? " mid" : " high";
  const natClass = peer.nat_ok ? "badge-ok" : "badge-warn";
  const natText = peer.nat_ok ? "P2P direct" : "NAT strict";
  card.innerHTML = `
    <div class="pc-top">
      <div class="pc-avatar">${initials}</div>
      <div class="pc-name" title="${peer.name}">${peer.name}</div>
    </div>
    <div class="pc-ip copyable" title="Click to copy">${peer.virtual_ip}</div>
    <div class="pc-meta">
      <span class="pc-ping${rttClass}">${rtt}</span>
      <span class="badge ${natClass} pc-nat">${natText}</span>
    </div>`;
  card.querySelector(".pc-ip").dataset.idx = idx;
  card.querySelector(".pc-ip").addEventListener("click", () => copyText(peer.virtual_ip));
  card.style.animationDelay = `${Math.min(idx * 40, 400)}ms`;
  return card;
}

function patchPeerCard(card, peer) {
  const rtt = peer.rtt_ms == null ? "—" : `${peer.rtt_ms}ms`;
  const rttClass = peer.rtt_ms == null ? "" : peer.rtt_ms <= 40 ? " ok" : peer.rtt_ms <= 120 ? " mid" : " high";
  const natClass = peer.nat_ok ? "badge-ok" : "badge-warn";
  const natText = peer.nat_ok ? "P2P direct" : "NAT strict";
  const pingEl = card.querySelector(".pc-ping");
  pingEl.className = "pc-ping" + rttClass;
  pingEl.textContent = rtt;
  card.querySelector(".pc-ip").textContent = peer.virtual_ip;
  const natEl = card.querySelector(".pc-nat");
  natEl.className = "badge " + natClass + " pc-nat";
  natEl.textContent = natText;
  if (peer.rtt_ms != null && peer.rtt_ms <= 40) {
    card.style.borderColor = "rgba(52,245,165,0.4)";
  }
}

function renderPeerMatrix() {
  const grid = $("peer-matrix");
  const names = Object.keys(peerCache);
  if (names.length === 0) {
    grid.querySelectorAll(".peer-card").forEach((el) => el.remove());
    $("empty-state").classList.remove("hidden");
    $("peer-count").textContent = "0 peers";
    grid.dataset.count = "0";
    netCanvas.schedule();
    return;
  }
  $("empty-state").classList.add("hidden");
  $("peer-count").textContent = `${names.length} peer${names.length === 1 ? "" : "s"}`;
  grid.dataset.count = String(names.length);

  let idx = 0;
  for (const peer of Object.values(peerCache)) {
    const existing = grid.querySelector(`.peer-card[data-name="${CSS.escape(peer.name)}"]`);
    if (existing) patchPeerCard(existing, peer);
    else grid.appendChild(buildPeerCard(peer, idx));
    idx++;
  }
  netCanvas.schedule();

  const want = new Set(names);
  grid.querySelectorAll(".peer-card").forEach((el) => {
    if (!want.has(el.dataset.name)) el.remove();
  });
  netCanvas.schedule();
}

async function refreshPeers() {
  try {
    const peers = await invoke("peers_snapshot");
    const next = {};
    for (const p of peers) next[p.name] = { ...p };
    peerCache = next;
    renderPeerMatrix();
  } catch { /* not in a room yet */ }
}

// ── Peer graph canvas ────────────────────────
const netCanvas = (() => {
  const cv = $("net-canvas");
  if (!cv || matchMedia("(prefers-reduced-motion: reduce)").matches) return { schedule() {} };
  const ctx = cv.getContext("2d");
  let raf = 0;
  function resize() {
    const r = cv.parentElement.getBoundingClientRect();
    const d = window.devicePixelRatio || 1;
    cv.width = Math.max(1, Math.round(r.width * d));
    cv.height = Math.max(1, Math.round(r.height * d));
    ctx.setTransform(d, 0, 0, d, 0, 0);
  }
  function nodes() {
    return [...document.querySelectorAll("#peer-matrix .peer-card")].map((el) => {
      const r = el.getBoundingClientRect();
      const mr = cv.parentElement.getBoundingClientRect();
      return { x: r.left - mr.left + r.width / 2, y: r.top - mr.top + r.height / 2 };
    });
  }
  function draw(now, pts) {
    ctx.clearRect(0, 0, cv.width, cv.height);
    ctx.strokeStyle = "rgba(56,225,255,0.16)";
    ctx.lineWidth = 1;
    for (let i = 0; i < pts.length; i++) {
      for (let j = i + 1; j < pts.length; j++) {
        ctx.beginPath(); ctx.moveTo(pts[i].x, pts[i].y); ctx.lineTo(pts[j].x, pts[j].y); ctx.stroke();
      }
    }
    if (pts.length >= 2 && pts.length <= 8) {
      const t = (now / 1000) % 2.4;
      for (let i = 0; i < pts.length; i++) {
        const j = (i + 1) % pts.length;
        const k = ((t + i * 0.3) % 2.4) / 2.4;
        ctx.beginPath();
        ctx.arc(pts[i].x + (pts[j].x - pts[i].x) * k, pts[i].y + (pts[j].y - pts[i].y) * k, 2, 0, Math.PI * 2);
        ctx.fillStyle = "rgba(56,225,255,0.7)"; ctx.fill();
      }
    }
  }
  function tick(now) {
    const pts = nodes();
    if (pts.length >= 2) draw(now, pts);
    else ctx.clearRect(0, 0, cv.width, cv.height);
    raf = requestAnimationFrame(tick);
  }
  resize();
  raf = requestAnimationFrame(tick);
  window.addEventListener("resize", resize);
  return { schedule() { if (raf) cancelAnimationFrame(raf); raf = requestAnimationFrame(tick); } };
})();

// ── Live events ─────────────────────────────
listen("peer-joined", () => refreshPeers());
listen("peer-left", () => refreshPeers());
listen("peer-list", () => refreshPeers());

listen("ping-update", (event) => {
  const [name, rtt] = event.payload;
  if (peerCache[name]) {
    peerCache[name].rtt_ms = rtt;
    const el = document.querySelector(`.peer-card[data-name="${CSS.escape(name)}"] .pc-ping`);
    if (el) {
      const cls = rtt <= 40 ? " ok" : rtt <= 120 ? " mid" : " high";
      el.className = "pc-ping" + cls;
      el.textContent = `${rtt}ms`;
      el.classList.remove("pc-ping-flash");
      void el.offsetWidth;
      el.classList.add("pc-ping-flash");
      const cardEl = document.querySelector(`.peer-card[data-name="${CSS.escape(name)}"]`);
      if (cardEl && rtt <= 40) {
        cardEl.style.borderColor = "rgba(109,223,154,0.4)";
      }
    }
  }
});

listen("nat-status", (event) => {
  $("nat-badge").textContent = "Strict";
  $("nat-badge").className = "badge badge-warn";
  toast(`NAT: ${event.payload}`);
});

// ── Bootstrapped refresh ────────────────────
async function ready() {
  await refreshPeers();
  try {
    const info = await invoke("room_info");
    if (info) {
      showInRoom(info.id, info.my_vip);
      if (info.host && info.host !== "player") $("host-name").textContent = info.host;
    }
  } catch { /* not in room */ }
  checkAdmin();
}

document.addEventListener("DOMContentLoaded", ready);
setInterval(refreshPeers, 3000);