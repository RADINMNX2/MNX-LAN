(function () {
  "use strict";

  const IS_TAURI = typeof window !== "undefined" && "__TAURI__" in window;
  document.documentElement.classList.add(IS_TAURI ? "in-tauri" : "in-browser");

  const STATE = {
    mode: "idle",
    room: "",
    vip: "",
    public: "",
    selfName: "",
    adapter: false,
    port: 38888,
    peers: [],
  };

  const $ = (id) => document.getElementById(id);
  const invoke = (cmd, args) =>
    IS_TAURI
      ? window.__TAURI__.core.invoke(cmd, args || {})
      : demoInvoke(cmd, args || {});
  const listen = (evt, cb) =>
    IS_TAURI ? window.__TAURI__.event.listen(evt, cb) : demoListen(evt, cb);

  function toast(level, message) {
    const box = $("toasts");
    const t = document.createElement("div");
    t.className = "toast toast-" + level;
    t.textContent = message;
    box.appendChild(t);
    requestAnimationFrame(() => t.classList.add("show"));
    setTimeout(() => {
      t.classList.remove("show");
      setTimeout(() => t.remove(), 320);
    }, 4200);
  }

  async function copyText(text) {
    try {
      await navigator.clipboard.writeText(text);
    } catch (_) {
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      try {
        document.execCommand("copy");
      } catch (_) {}
      ta.remove();
    }
    toast("success", "Copied to clipboard");
  }

  function windowApi() {
    return IS_TAURI ? window.__TAURI__.window.getCurrentWindow() : null;
  }

  function autosize() {
    const el = windowApi();
    if (el) {
      try {
        el.innerSize({ width: 1180, height: 760 });
      } catch (_) {}
    }
  }

  function setupWindowControls() {
    $("btn-close").addEventListener("click", () => {
      const el = windowApi();
      if (el) el.close();
      else window.close();
    });
    $("btn-min").addEventListener("click", () => {
      const el = windowApi();
      if (el) el.minimize();
    });
    $("btn-max").addEventListener("click", async () => {
      const el = windowApi();
      if (el) await el.toggleMaximize();
    });
  }

  function nav() {
    document.querySelectorAll(".nav-item").forEach((btn) => {
      btn.addEventListener("click", () => {
        const view = btn.dataset.view;
        document.querySelectorAll(".nav-item").forEach((b) => b.classList.toggle("active", b === btn));
        document.querySelectorAll(".view").forEach((v) => v.classList.toggle("active", v.id === "view-" + view));
      });
    });
  }

  function pingColor(ms) {
    if (ms === 0 || ms == null) return "idle";
    if (ms < 40) return "good";
    if (ms < 100) return "ok";
    return "bad";
  }

  function nameHash(name) {
    let h = 0;
    for (const c of name || "") h = (h * 31 + c.charCodeAt(0)) >>> 0;
    return h % 6;
  }

  const AVATAR_HUES = [207, 263, 158, 16, 324, 47];

  function renderSelf() {
    const card = $("self-card");
    const hue = AVATAR_HUES[nameHash(STATE.selfName || "me")];
    card.innerHTML =
      '<div class="avatar" style="--h:' + hue + '">' +
      (STATE.adapter ? '<span class="ring-online"></span>' : "") +
      esc(STATE.selfName[0] || "?") +
      "</div>" +
      '<div class="peer-info"><div class="peer-name">' +
      esc(STATE.selfName || "You") +
      ' <span class="tag-self">you</span></div>' +
      '<button class="vip-chip" data-copy="' +
      (STATE.vip || "10.88.0.1") +
      '">' +
      esc(STATE.vip || "10.88.0.1") +
      ' <span class="copy-ico">⧉</span></button>' +
      '<div class="peer-sub">' +
      (STATE.adapter ? "Virtual adapter up" : "Adapter offline") +
      " · " +
      esc(STATE.mode === "host" ? "Host" : STATE.mode === "member" ? "Member" : "Idle") +
      "</div></div>";
  }

  function renderPeers() {
    const grid = $("peer-grid");
    const empty = $("peer-empty");
    const others = STATE.peers.filter((p) => p.id !== STATE.self_id);
    const badge = $("peer-count");
    badge.textContent = String(others.length);
    badge.hidden = others.length === 0;

    grid.querySelectorAll(".peer-card:not(.self)").forEach((n) => n.remove());
    others.forEach((peer) => {
      const hue = AVATAR_HUES[nameHash(peer.name)];
      const ping = peer.rtt_ms || 0;
      const p2p = !!peer.p2p;
      const card = document.createElement("div");
      card.className = "peer-card";
      card.innerHTML =
        '<div class="avatar" style="--h:' +
        hue +
        '">' +
        (p2p ? '<span class="ring-online"></span>' : "") +
        esc(peer.name[0] || "?") +
        "</div>" +
        '<div class="peer-info">' +
        '<div class="peer-name">' +
        esc(peer.name) +
        "</div>" +
        '<button class="vip-chip" data-copy="' +
        esc(peer.vip) +
        '">' +
        esc(peer.vip) +
        ' <span class="copy-ico">⧉</span></button>' +
        '<div class="peer-sub"><span class="status-dot dot-' +
        (p2p ? "good" : "warn") +
        '"></span>' +
        (p2p ? "Direct P2P" : "Punching…") +
        "</div>" +
        "</div>" +
        '<div class="ping ping-' +
        pingColor(ping) +
        '">' +
        (ping ? ping + " ms" : "–") +
        "</div>";
      grid.appendChild(card);
    });

    empty.hidden = others.length > 0;
  }

  function esc(s) {
    return String(s == null ? "" : s).replace(/[&<>"']/g, (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c])
    );
  }

  function applySession(s) {
    STATE.mode = s.mode || "idle";
    STATE.room = s.room || "";
    STATE.vip = s.vip || "";
    STATE.public = s.public || "";
    STATE.selfName = s.self_name || STATE.selfName;
    STATE.adapter = !!s.adapter;
    STATE.self_id = s.self_id;
    STATE.port = s.port || STATE.port;

    const status = $("net-status");
    const dot = $("status-dot");
    $("status-text").textContent =
      STATE.mode === "host" ? "Hosting · " + STATE.room : STATE.mode === "member" ? "In " + STATE.room : "Idle";
    dot.className = "dot dot-" + (STATE.mode === "idle" ? "idle" : "good");

    $("room-code").textContent = STATE.room || "MNX-XXXX";
    $("room-vip").textContent = STATE.vip || "—";
    $("room-pub").textContent = STATE.public || "—";
    const invite = STATE.public && STATE.public !== "0.0.0.0:0" ? STATE.room + "@" + STATE.public : STATE.public ? STATE.room + " (local LAN)" : "—";
    $("room-invite").textContent = invite;
    $("room-banner").hidden = STATE.mode === "idle";
    $("room-empty").hidden = STATE.mode !== "idle";
    $("room-subtitle").textContent =
      STATE.mode === "idle"
        ? "Connection &amp; identity at a glance."
        : "Everyone in " + STATE.room + " tunnels directly to each other.";

    $("kv-adapter").textContent = STATE.adapter ? "Online" : "Offline";
    $("kv-vip").textContent = STATE.vip || "—";
    $("kv-mode").textContent =
      STATE.mode === "host" ? "Host · " + STATE.room : STATE.mode === "member" ? "Member · " + STATE.room : "Idle";
    $("set-name").value = STATE.selfName;
    renderSelf();
  }

  async function refreshSession() {
    try {
      const s = await invoke("get_session");
      applySession(s);
    } catch (e) {
      console.error(e);
    }
  }

  async function refreshPeers() {
    try {
      const p = await invoke("get_peers");
      STATE.peers = p.peers || [];
      renderPeers();
    } catch (e) {
      console.error(e);
    }
  }

  function wireActions() {
    $("btn-create").addEventListener("click", async () => {
      const name = $("host-name").value.trim() || STATE.selfName || "Player";
      $("btn-create").classList.add("loading");
      $("btn-create").disabled = true;
      try {
        const s = await invoke("create_room", { name });
        applySession(s);
        navTo("rooms");
      } catch (e) {
        toast("error", String(e));
      } finally {
        $("btn-create").classList.remove("loading");
        $("btn-create").disabled = false;
      }
    });
    $("host-name").addEventListener("keydown", (e) => {
      if (e.key === "Enter") $("btn-create").click();
    });

    $("btn-join").addEventListener("click", async () => {
      const target = $("join-target").value.trim();
      if (!target) {
        toast("error", "Enter a room code (MNX-XXXX) or connect string.");
        return;
      }
      $("btn-join").classList.add("loading");
      $("btn-join").disabled = true;
      try {
        const s = await invoke("join_room", { target });
        applySession(s);
        navTo("peers");
      } catch (e) {
        toast("error", String(e));
      } finally {
        $("btn-join").classList.remove("loading");
        $("btn-join").disabled = false;
      }
    });
    $("join-target").addEventListener("keydown", (e) => {
      if (e.key === "Enter") $("btn-join").click();
    });

    $("btn-leave").addEventListener("click", async () => {
      await invoke("leave_room");
      STATE.peers = [];
      renderPeers();
      refreshSession();
      navTo("home");
    });

    $("btn-save-name").addEventListener("click", async () => {
      const name = $("set-name").value.trim();
      if (name) {
        await invoke("set_name", { name });
        STATE.selfName = name;
        renderSelf();
        toast("success", "Name updated");
      }
    });

    document.addEventListener("click", (e) => {
      const chip = e.target.closest("[data-copy]");
      if (chip) copyText(chip.dataset.copy);
    });
  }

  function navTo(view) {
    document.querySelectorAll(".nav-item").forEach((b) => b.classList.toggle("active", b.dataset.view === view));
    document.querySelectorAll(".view").forEach((v) => v.classList.toggle("active", v.id === "view-" + view));
  }

  function applyWindowEffects() {
    if (!IS_TAURI) return;
    try {
      const api = window.__TAURI__.window;
      if (api && api.getCurrentWindow && api.getCurrentWindow().setEffects) {
        api.getCurrentWindow().setEffects({
          effects: [
            {
              variant: "mica",
              radius: 12,
            },
          ],
          state: "active",
        });
      }
    } catch (_) {}
  }

  function init() {
    setupWindowControls();
    nav();
    wireActions();
    applyWindowEffects();
    listen("session", (e) => applySession(e.payload));
    listen("peers", (e) => {
      STATE.peers = (e.payload && e.payload.peers) || [];
      renderPeers();
    });
    listen("notify", (e) => {
      if (e.payload) toast(e.payload.level || "info", e.payload.message || "");
    });
    autosize();
    refreshSession().then(() => refreshPeers());
  }

  /* ---------------- browser demo fallback ---------------- */

  let demoPeers = [];
  function demoInvoke(cmd, args) {
    if (args && args.name !== undefined) STATE.selfName = args.name;
    if (cmd === "get_session") {
      return Promise.resolve({
        mode: STATE.mode,
        room: STATE.room,
        vip: STATE.vip,
        public: STATE.public,
        self_name: STATE.selfName,
        adapter: STATE.adapter,
        self_id: "mx-demo000000000",
        port: 38888,
      });
    }
    if (cmd === "create_room") {
      STATE.mode = "host";
      STATE.room = "MNX-" + rand4();
      STATE.vip = "10.88.0.1";
      STATE.public = "203.0.113." + (10 + Math.floor(Math.random() * 200)) + ":38888";
      STATE.adapter = true;
      demoPeers = [
        demoPeer("Kai", "10.88.0.2"),
        demoPeer("Ivy", "10.88.0.3"),
      ];
      setTimeout(() => STATE.peers = demoPeers, 800);
      return Promise.resolve(getSessionState());
    }
    if (cmd === "join_room") {
      STATE.mode = "member";
      STATE.room = String(args.target).split("@")[0].toUpperCase();
      STATE.vip = "10.88.0." + (2 + Math.floor(Math.random() * 20));
      STATE.public = "198.51.100." + (10 + Math.floor(Math.random() * 200)) + ":" + STATE.port;
      STATE.adapter = true;
      demoPeers = [demoPeer("Dav", "10.88.0.1"), demoPeer("Nox", "10.88.0.4")];
      setTimeout(() => STATE.peers = demoPeers, 800);
      return Promise.resolve(getSessionState());
    }
    if (cmd === "leave_room") {
      STATE.mode = "idle";
      STATE.room = "";
      STATE.vip = "";
      STATE.public = "";
      STATE.peers = [];
      demoPeers = [];
      return Promise.resolve();
    }
    if (cmd === "get_peers") return Promise.resolve({ peers: STATE.peers });
    if (cmd === "set_name") return Promise.resolve();
    return Promise.resolve({});
  }

  function getSessionState() {
    return {
      mode: STATE.mode,
      room: STATE.room,
      vip: STATE.vip,
      public: STATE.public,
      self_name: STATE.selfName,
      adapter: STATE.adapter,
      self_id: "mx-demo000000000",
      port: STATE.port,
    };
  }

  function demoPeer(name, vip) {
    return {
      id: "mx-demo" + Math.random().toString(16).slice(2, 8),
      name,
      vip,
      public: "203.0.113.7:38888",
      rtt_ms: 12 + Math.floor(Math.random() * 60),
      p2p: true,
    };
  }

  function demoListen(evt, cb) {
    if (evt === "peers") setInterval(() => { cb({ payload: { peers: STATE.peers } }); }, 2200);
    return Promise.resolve({});
  }

  function rand4() {
    return "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"
      .split("")
      .sort(() => Math.random() - 0.5)
      .slice(0, 4)
      .join("");
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();