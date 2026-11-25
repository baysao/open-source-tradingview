// TradeX sync: keeps the allow-listed localStorage keys identical across a user's browsers.
// Loaded FIRST on every page (before theme.js) so the Storage wrapper is installed before any
// reader or writer runs. Plain script, no bundler, no framework; attaches window.VtvSync.
//
// Contract: /api/me/state as shipped by 05-02. The stored value is the exact string the page
// wrote (JSON.stringify output): this file never parses or re-serialises a synced value, because
// a JSON.parse + JSON.stringify round trip would reformat every drawing file and manufacture a
// conflict on the next compare.
(() => {
  "use strict";
  if (window.VtvSync) return;                        // a page that loads this script twice
  if (Storage.prototype.__vtv) return;

  // ---------- the allow-list ----------
  // R16 names two tradex keys, not the tradex namespace, so no wildcard.
  const EXACT = new Set([
    "chartHub", "chartHubDraft", "chartHubFavs", "chartHubIndicators",
    "chartHubScripts", "chartHubTheme", "tradexDrawFavs", "tradexFavBar",
  ]);
  // A placeholder for a future Vela persistence option: if it lands, its key is already allowed.
  const PREFIXES = ["vela:"];

  const isAllowed = (k) =>
    EXACT.has(k) || PREFIXES.some((p) => k.startsWith(p));
  // vtv: is the version cache and backup: is written by the server-side merge; neither is synced.
  const isExcluded = (k) => k.startsWith("vtv:") || k.startsWith("backup:");
  const allowed = (k) => typeof k === "string" && k !== "" && !isExcluded(k) && isAllowed(k);

  // ---------- constants ----------
  const DEBOUNCE_MS = 2000;
  const RETRY_MS = [1000, 2000, 4000, 8000, 15000];
  const RETRY_JITTER = 0.2;
  const MAX_ATTEMPTS = 6;
  const VER_PREFIX = "vtv:ver:";
  const MSG_TAG = 1;
  const IS_CHILD = window.parent !== window;

  // ---------- storage, through the originals ----------
  // Captured before the wrapper is installed: raw.* must never route through the patched
  // prototype, or a version write would queue itself and a downloaded value would echo back.
  const origGet = Storage.prototype.getItem;
  const origSet = Storage.prototype.setItem;
  const origRemove = Storage.prototype.removeItem;
  const LS = () => window.localStorage;
  const raw = {
    get: (k) => { try { return origGet.call(LS(), k); } catch (_) { return null; } },
    set: (k, v) => { try { origSet.call(LS(), k, v); } catch (_) {} },
    del: (k) => { try { origRemove.call(LS(), k); } catch (_) {} },
    keys: () => { try { return Object.keys(LS()); } catch (_) { return []; } },
  };

  const getVersion = (k) => {
    const v = parseInt(raw.get(VER_PREFIX + k), 10);
    return Number.isFinite(v) ? v : null;
  };
  const setVersion = (k, v) => raw.set(VER_PREFIX + k, String(v));

  // ---------- state ----------
  const state = {
    signedIn: false,
    mergedOnce: false,
    online: true,
  };
  const slots = new Map();   // key -> {kind, value, timer, attempts, state}
  const listeners = new Map(); // event -> Set<fn>
  const inflight = new Map();  // key -> promise, so one key never has two PUTs
  let readyResolve;
  const ready = new Promise((res) => { readyResolve = res; });
  let readySettled = false;

  const emit = (type, payload) => {
    const set = listeners.get(type);
    if (!set) return;
    for (const fn of [...set]) { try { fn(payload); } catch (_) {} }
  };

  const slot = (k) => {
    let s = slots.get(k);
    if (!s) { s = { kind: "set", value: undefined, timer: 0, attempts: 0, state: "idle" }; slots.set(k, s); }
    return s;
  };

  // The status line reads one word: conflict outranks saving outranks offline.
  function statusOf() {
    let saving = false, offline = false, conflict = false;
    for (const s of slots.values()) {
      if (s.state === "conflict") conflict = true;
      else if (s.state === "dirty" || s.state === "inflight") saving = true;
      else if (s.state === "unsynced" || s.state === "retry") offline = true;
    }
    if (conflict) return "conflict";
    if (saving) return "saving";
    if (offline) return "offline";
    return "saved";
  }
  function publishStatus() {
    emit("status", { state: statusOf(), at: new Date().toISOString() });
  }

  // ---------- the wrapper ----------
  Object.defineProperty(Storage.prototype, "__vtv", { value: true, enumerable: false, configurable: false });

  function note(k, kind, value) {
    // sessionStorage shares Storage.prototype; only localStorage is the synced surface.
    if (!allowed(k)) return;
    if (IS_CHILD) {
      // One writer per tab: the iframe never PUTs, it forwards. No debounce here — the parent
      // owns the per-key debounce, so a burst of child writes collapses into one message trail
      // and one PUT.
      post(window.parent, kind === "del" ? { t: "del", key: k } : { t: "set", key: k, value });
      return;
    }
    const s = slot(k);
    s.kind = kind;
    s.value = value;
    s.state = "dirty";
    clearTimeout(s.timer);
    s.timer = setTimeout(() => flush(k), DEBOUNCE_MS);
    publishStatus();
  }

  Storage.prototype.setItem = function setItem(k, v) {
    // The original runs FIRST: a quota error must still throw to the caller exactly as before,
    // and the write must land even when sync is off.
    origSet.call(this, k, v);
    if (this !== window.localStorage) return;
    note(String(k), "set", String(v));
  };
  Storage.prototype.removeItem = function removeItem(k) {
    origRemove.call(this, k);
    if (this !== window.localStorage) return;
    note(String(k), "del", undefined);
  };

  // ---------- the channel ----------
  const TRUSTED = (() => {
    try { return typeof Origin === "function" ? Origin.from(location.origin) : null; } catch (_) { return null; }
  })();

  // Origin.from(ev) reads the internal origin a real postMessage carries; a synthetic
  // MessageEvent only has the `origin` string and throws there, so the nested fallback is the
  // shape the guide documents. No Origin support means reject, not a string compare.
  function fromTrustedOrigin(ev, expectedSource) {
    if (expectedSource && ev.source !== expectedSource) return false;
    if (!TRUSTED) return false;
    try { if (Origin.from(ev).isSameOrigin(TRUSTED)) return true; } catch (_) {}
    try { if (Origin.from(ev.origin).isSameOrigin(TRUSTED)) return true; } catch (_) {}
    return false;
  }

  const frameWindow = () => {
    const el = document.getElementById("frame");
    try { return (el && el.contentWindow) || null; } catch (_) { return null; }
  };
  const expectedSource = () => (IS_CHILD ? window.parent : frameWindow());

  function post(target, msg) {
    if (!target) return;
    try { target.postMessage({ vtv: MSG_TAG, ...msg }, location.origin); } catch (_) {}
  }

  let reloaded = false;
  function reloadFrameOnce() {
    if (reloaded) return;
    reloaded = true;
    if (IS_CHILD) { location.reload(); return; }
    const el = document.getElementById("frame");
    try { if (el) el.src = el.src; } catch (_) {}
  }

  const versions = () => {
    const out = {};
    for (const k of Object.keys(raw.keys())) {
      if (!k.startsWith(VER_PREFIX)) continue;
      const key = k.slice(VER_PREFIX.length);
      if (!allowed(key)) continue;
      const v = getVersion(key);
      if (v != null) out[key] = v;
    }
    return out;
  };

  window.addEventListener("message", (ev) => {
    if (!fromTrustedOrigin(ev, expectedSource())) return;
    const d = ev.data;
    if (!d || d.vtv !== MSG_TAG) return;
    if (IS_CHILD) {
      // parent -> child
      if (d.t === "state") {
        setSignedIn(!!d.signedIn, false);
        settleReady();
      } else if (d.t === "reload") {
        reloadFrameOnce();
      }
      return;
    }
    // child -> parent
    if (d.t === "hello") {
      post(frameWindow(), { t: "state", signedIn: state.signedIn, versions: versions() });
    } else if (d.t === "set" && allowed(d.key)) {
      note(d.key, "set", String(d.value));
    } else if (d.t === "del" && allowed(d.key)) {
      note(d.key, "del", undefined);
    } else if (d.t === "need-reload") {
      reloadFrameOnce();
    }
  });

  // ---------- the API calls ----------
  async function api(method, path, { body, ifMatch } = {}) {
    const headers = {};
    if (body !== undefined) headers["Content-Type"] = "application/json";
    if (ifMatch !== undefined) headers["If-Match"] = String(ifMatch);
    let res;
    try {
      res = await fetch(path, {
        method,
        credentials: "same-origin",
        cache: "no-store",
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
      });
    } catch (err) {
      return { ok: false, network: true, status: 0, error: String(err && err.message || err) };
    }
    let json = null;
    try { json = res.status === 204 ? null : await res.json(); } catch (_) {}
    return {
      ok: res.ok,
      status: res.status,
      json,
      error: json && json.error ? String(json.error) : "",
    };
  }

  const putValue = (k, value, ifMatch) =>
    api("PUT", `/api/me/state/${encodeURIComponent(k)}`, { body: { value }, ifMatch });

  // ---------- the state machine ----------
  const backoff = (attempt) => {
    const base = RETRY_MS[Math.min(attempt, RETRY_MS.length - 1)];
    const jitter = base * RETRY_JITTER * (Math.random() * 2 - 1);
    return Math.max(250, Math.round(base + jitter));
  };

  function scheduleRetry(k) {
    const s = slot(k);
    s.attempts += 1;
    if (s.attempts > MAX_ATTEMPTS) {
      s.state = "unsynced";
      publishStatus();
      emit("status", { state: "offline", at: new Date().toISOString(), key: k, reason: "retries-exhausted" });
      return;
    }
    s.state = "retry";
    clearTimeout(s.timer);
    s.timer = setTimeout(() => { s.state = "dirty"; flush(k); }, backoff(s.attempts - 1));
    publishStatus();
  }

  async function flush(k) {
    const s = slots.get(k);
    if (!s) return;
    if (!state.signedIn) return;                    // parked until sign-in state changes
    if (s.state === "conflict" || s.state === "unsynced") return;
    if (inflight.has(k)) return;
    const value = s.value;
    const kind = s.kind;
    const job = (async () => {
      s.state = "inflight";
      publishStatus();
      const version = getVersion(k);
      const res = kind === "del"
        ? await api("DELETE", `/api/me/state/${encodeURIComponent(k)}`)
        : await putValue(k, value, version == null ? 0 : version);

      if (res.ok) {
        if (kind === "del") raw.del(VER_PREFIX + k);
        else if (res.json && typeof res.json.version === "number") setVersion(k, res.json.version);
        s.attempts = 0;
        s.state = "idle";
        publishStatus();
        emit("saved", { key: k, at: new Date().toISOString() });
        return;
      }
      if (res.status === 409) {
        const err = res.error;
        if (err === "version conflict") {
          // A conflict is a decision, not a failure: never retried, and the local value stays
          // readable until the user chooses.
          s.state = "conflict";
          publishStatus();
          emit("conflict", {
            key: k,
            mine: value,
            theirs: res.json ? res.json.value : null,
            version: res.json ? res.json.version : null,
            at: new Date().toISOString(),
            accept: () => acceptTheirs(k, res.json),
            keep: () => keepMine(k, version),
          });
          return;
        }
        // "too many keys": the account is full. Report it, do not retry it.
        s.state = "unsynced";
        publishStatus();
        emit("status", { state: "offline", at: new Date().toISOString(), key: k, reason: "too-many-keys" });
        return;
      }
      if (res.status === 413) {
        // Too large to sync: kept in this browser for the rest of the session.
        s.state = "unsynced";
        publishStatus();
        emit("status", { state: "offline", at: new Date().toISOString(), key: k, reason: "too-large" });
        return;
      }
      if (res.status === 401) {
        state.signedIn = false;
        s.state = "dirty";
        publishStatus();
        return;
      }
      scheduleRetry(k);
    })();
    inflight.set(k, job);
    try { await job; } finally { inflight.delete(k); }
  }

  function acceptTheirs(k, body) {
    const s = slot(k);
    if (body && typeof body.value === "string") raw.set(k, body.value);
    if (body && typeof body.version === "number") setVersion(k, body.version);
    s.state = "idle";
    s.attempts = 0;
    publishStatus();
    reloadFrameOnce();
  }

  async function keepMine(k, version) {
    const s = slot(k);
    const value = s.value;
    s.state = "dirty";
    const res = await putValue(k, value, version == null ? 0 : version);
    if (res.ok) {
      if (res.json && typeof res.json.version === "number") setVersion(k, res.json.version);
      s.state = "idle";
      s.attempts = 0;
      publishStatus();
      emit("saved", { key: k, at: new Date().toISOString() });
      return;
    }
    if (res.status === 409 && res.error === "version conflict") {
      // A raced "Keep mine": another device advanced the version between the conflict and the
      // click. Retrying cannot succeed — the PUT will 409 forever. This is a fresh decision,
      // not a failure: re-enter the conflict with the new body.
      s.state = "conflict";
      publishStatus();
      emit("conflict", {
        key: k,
        mine: value,
        theirs: res.json ? res.json.value : null,
        version: res.json ? res.json.version : null,
        at: new Date().toISOString(),
        accept: () => acceptTheirs(k, res.json),
        keep: () => keepMine(k, res.json ? res.json.version : null),
      });
      return;
    }
    scheduleRetry(k);
  }

  // ---------- the merge, on the transition to signed in ----------
  const BACKUP_STAMP = () => {
    const d = new Date();
    const p = (n) => String(n).padStart(2, "0");
    // UTC, `-` between date and time so two same-day backups never collide (R23).
    return `${d.getUTCFullYear()}-${p(d.getUTCMonth() + 1)}-${p(d.getUTCDate())}-` +
           `${p(d.getUTCHours())}${p(d.getUTCMinutes())}${p(d.getUTCSeconds())}`;
  };
  const backupName = (k) => `backup:${k}:${BACKUP_STAMP()}`;
  const KEY_FROM_BACKUP = (bk) =>
    bk.replace(/^backup:/, "").replace(/:\d{4}-\d{2}-\d{2}-\d{6}$/, "");

  async function merge() {
    const idx = await api("GET", "/api/me/state");
    if (!idx.ok) return;
    const server = new Map();
    for (const e of (idx.json && idx.json.keys) || []) {
      if (e && allowed(e.key)) server.set(e.key, e);
    }
    const localKeys = raw.keys().filter(allowed);
    const localSet = new Set(localKeys);
    const backups = [];
    const notBackedUp = [];
    const adopted = [];

    // 1. A key on both sides with a different value: back the local copy up FIRST, then adopt
    // the server's. Order matters: a crash mid-merge then loses nothing but an extra backup.
    for (const k of localKeys) {
      const entry = server.get(k);
      if (!entry) continue;
      const local = raw.get(k);
      if (local == null) continue;
      const one = await api("GET", `/api/me/state/${encodeURIComponent(k)}`);
      if (!one.ok || !one.json || typeof one.json.value !== "string") continue;
      if (one.json.value === local) { setVersion(k, Number(entry.version)); continue; }  // AC-7: no backup for equality
      const bk = backupName(k);
      const saved = await putValue(bk, local, 0);
      if (!saved.ok) {
        // 409 "too many keys" (or a same-second two-tab backup-stamp collision): the plan's
        // contract is that an unbacked key KEEPS ITS LOCAL VALUE — adopting the server's here
        // would destroy work with no backup. Report it and leave the key parked.
        notBackedUp.push(k);
        continue;
      }
      backups.push({ backup: bk, key: k, at: new Date().toISOString() });
      raw.set(k, one.json.value);
      setVersion(k, Number(one.json.version));
      adopted.push(k);
    }

    // 2. Keys only on the server: write through the ORIGINAL setItem, so downloading a value
    // does not queue a sync of it and echo the server back to itself.
    for (const [k, entry] of server) {
      if (localSet.has(k)) continue;
      const one = await api("GET", `/api/me/state/${encodeURIComponent(k)}`);
      if (!one.ok || !one.json || typeof one.json.value !== "string") continue;
      raw.set(k, one.json.value);
      setVersion(k, Number(one.json.version));
      adopted.push(k);
    }

    // 3. Keys only locally: upload as an insert — but only keys with NO pending write. A key
    // sitting dirty in the state machine already owns its upload (the debounced flush will fire
    // with the right If-Match); the merge racing it would double-PUT with a stale snapshot.
    for (const k of localKeys) {
      if (server.has(k)) continue;
      const s = slots.get(k);
      if (s && (s.state === "dirty" || s.state === "inflight" || s.state === "retry")) continue;
      const local = raw.get(k);
      if (local == null) continue;
      const res = await putValue(k, local, 0);
      if (res.ok && res.json && typeof res.json.version === "number") setVersion(k, res.json.version);
      else if (res.status === 413 || (res.status === 409 && res.error === "too many keys")) notBackedUp.push(k);
    }

    if (adopted.length) reloadFrameOnce();          // AC-9: one reload, not one per key
    if (backups.length || notBackedUp.length) {
      emit("backups", { backups, notBackedUp, at: new Date().toISOString() });
    }
    publishStatus();
  }

  function setSignedIn(next, runMerge) {
    if (state.signedIn === next) {
      if (next && runMerge && !state.mergedOnce) merge();
      return;
    }
    state.signedIn = next;
    if (!next) { publishStatus(); return; }
    // mergedOnce flips synchronously: HubAuth.onChange fires immediately when auth is already
    // resolved, and me.then fires right after it. Both would call merge() before the first one
    // (async) had finished, and two racing merges double-PUT every key.
    state.mergedOnce = true;
    // Re-arm every parked write, then merge. A 401 parks the queue; signing in resumes it.
    for (const [k, s] of slots) if (s.state === "dirty") flush(k);
    if (runMerge) merge();
    publishStatus();
  }

  function settleReady() {
    if (readySettled) return;
    readySettled = true;
    readyResolve({ signedIn: state.signedIn });
  }

  // ---------- sign-in state: auth.js owns it, this file reads it ----------
  function watchAuth() {
    if (IS_CHILD) {
      // The child makes zero requests: ask the parent over the channel.
      post(window.parent, { t: "hello" });
      setTimeout(() => settleReady(), 1500);
      return;
    }
    if (window.HubAuth && window.HubAuth.me) return useHubAuth();
    // sync.js is the first script in the document, so auth.js (defer) has not run yet. Wait for
    // it instead of fetching /auth/me a second time: a page that owns auth already asks once,
    // and two reads of the same endpoint is a duplicate request for no new information.
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", () => {
        if (window.HubAuth && window.HubAuth.me) useHubAuth();
        else readMeDirectly();
      }, { once: true });
      return;
    }
    readMeDirectly();
  }

  function useHubAuth() {
    // Hosted signed-out is {"auth": true, "user": null}: the auth flag says the server has auth
    // ON, the user object says the visitor signed in (the convention auth.js's render uses).
    Promise.resolve(window.HubAuth.me)
      .then((s) => { setSignedIn(!!(s && s.auth && s.user), true); settleReady(); })
      .catch(() => { settleReady(); });
    if (typeof window.HubAuth.onChange === "function") {
      window.HubAuth.onChange((s) => setSignedIn(!!(s && s.auth && s.user), true));
    }
  }

  // A page with no auth.js (a standalone engine page opened directly).
  function readMeDirectly() {
    fetch("/auth/me", { cache: "no-store", credentials: "same-origin" })
      .then((r) => (r.ok ? r.json() : { auth: false }))
      .catch(() => ({ auth: false }))
      .then((s) => { setSignedIn(!!(s && s.auth && s.user), true); settleReady(); });
  }

  // ---------- public surface ----------
  window.VtvSync = {
    ready,
    get signedIn() { return state.signedIn; },
    version: getVersion,
    on(event, fn) {
      if (typeof fn !== "function") return () => {};
      let set = listeners.get(event);
      if (!set) { set = new Set(); listeners.set(event, set); }
      set.add(fn);
      return () => set.delete(fn);
    },
    async restore(backupKey) {
      if (!allowed(KEY_FROM_BACKUP(backupKey))) return false;
      const one = await api("GET", `/api/me/state/${encodeURIComponent(backupKey)}`);
      if (!one.ok || !one.json || typeof one.json.value !== "string") return false;
      const k = KEY_FROM_BACKUP(backupKey);
      raw.set(k, one.json.value);
      const ver = getVersion(k);
      const res = await putValue(k, one.json.value, ver == null ? 0 : ver);
      if (res.ok && res.json && typeof res.json.version === "number") setVersion(k, res.json.version);
      publishStatus();
      reloadFrameOnce();
      return !!res.ok;
    },
    retry() {
      for (const [k, s] of slots) {
        if (s.state !== "retry" && s.state !== "unsynced") continue;
        s.attempts = 0;
        s.state = "dirty";
        flush(k);
      }
      publishStatus();
    },
  };

  // ---------- the notice (mockup 02, parent page only, DOM optional in tests) ----------
  const C7_TOO_LARGE = "Too large to sync: {key}, kept in this browser";
  const C7_MANY = "This account is full: {key} was not backed up, its local copy is kept";

  function noticeDom() {
    if (IS_CHILD || typeof document === "undefined") return null;
    const host = document.getElementById("syncNotices");
    return host || null;
  }

  const ariaNotify = (text) => {
    try { if (typeof window.ariaNotify === "function") window.ariaNotify(text); } catch (_) {}
  };

  function buildNotice({ role, icon, cls, title, body, actions, keys }) {
    const host = noticeDom();
    if (!host) return null;
    const n = document.createElement("div");
    n.className = "notice sync-n" + (cls ? " " + cls : "");
    n.setAttribute("role", role);
    const svg = `<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${icon}</svg>`;
    const html = [svg, "<div>", `<h3>${title}</h3>`, body ? `<p>${body}</p>` : ""];
    if (keys && keys.length) {
      html.push('<div class="keys" role="list" aria-label="Backups">');
      for (const k of keys) {
        html.push(
          k.restored
            ? `<div class="key done" role="listitem"><b>${k.name}</b><span>Restored: this browser's copy is current again</span></div>`
            : `<div class="key" role="listitem"><b>${k.name}</b><span>${k.note}</span>` +
              `<button type="button" class="btn" data-restore="${k.backup || ""}" data-key="${k.name}">Restore</button></div>`
        );
      }
      html.push("</div>");
    }
    if (actions && actions.length) {
      html.push('<div class="acts">');
      for (const a of actions) {
        html.push(`<button type="button" class="btn${a.primary ? " primary" : ""}" data-sync-act="${a.id}">${a.label}</button>`);
      }
      html.push("</div>");
    }
    html.push("</div>");
    n.innerHTML = html.join("");
    host.appendChild(n);
    const visibleText = [title, body || "", ...(keys || []).map((k) => k.note || "")].filter(Boolean).join(". ");
    ariaNotify(visibleText + (actions && actions.length ? ". Actions available" : ""));
    n.querySelectorAll("[data-restore]").forEach((b) => {
      b.addEventListener("click", async () => {
        const ok = await window.VtvSync.restore(b.dataset.restore);
        if (ok) {
          b.closest(".key").classList.add("done");
          b.remove();
        }
      });
    });
    n.querySelectorAll("[data-sync-act]").forEach((b) => {
      b.addEventListener("click", () => {
        const a = (actions || []).find((x) => x.id === b.dataset.syncAct);
        if (a && a.run) a.run();
      });
    });
    const dismiss = document.createElement("button");
    dismiss.type = "button";
    dismiss.className = "ghost";
    dismiss.textContent = "Dismiss";
    dismiss.addEventListener("click", () => n.remove());
    n.querySelector("div").appendChild(dismiss);
    return n;
  }

  const ICONS = {
    triangle: '<path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><path d="M12 9v4"/><path d="M12 17h.01"/>',
    cloudOff: '<path d="M3 3l18 18"/><path d="M17.5 19a4.5 4.5 0 0 0 .42-8.98 6 6 0 0 0-4.2-4.37"/><path d="M9.58 6.12A6 6 0 0 0 6.22 11.6 4 4 0 0 0 6 19.5h11.5"/>',
    check: '<path d="M20 6L9 17l-5-5"/>',
  };

  const when = (iso) => {
    const d = iso ? new Date(iso) : new Date();
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  };

  const on = window.VtvSync.on;   // shorthand for the notice wiring below

  // One notice PER KEY: a second key's conflict must never orphan the first key's notice —
  // the first key would stay parked in `conflict` with no UI path to resolve it.
  const conflictNotices = new Map(); // key -> notice element
  on("conflict", (c) => {
    const prior = conflictNotices.get(c.key);
    if (prior) prior.remove();
    const n = buildNotice({
      role: "alert",
      cls: "warn",
      icon: ICONS.triangle,
      title: `Another device saved this layout at ${when(c.at)}`,
      body: "Your change on this browser was not saved. Choose which version to keep: the other one is not lost until you choose.",
      actions: [
        { id: "load", label: "Load the other device's version", primary: true,
          run: () => { if (c.accept) c.accept(); const el = conflictNotices.get(c.key); if (el) el.remove(); conflictNotices.delete(c.key); } },
        { id: "keep", label: "Keep mine",
          run: () => { if (c.keep) c.keep(); const el = conflictNotices.get(c.key); if (el) el.remove(); conflictNotices.delete(c.key); } },
      ],
    });
    if (n) conflictNotices.set(c.key, n);
  });

  on("backups", (b) => {
    const keys = [
      ...(b.backups || []).map((x) => ({
        name: x.key,
        note: `Backup of ${when(x.at)}, expires in 30 days`,
        backup: x.backup,
      })),
      ...(b.notBackedUp || []).map((k) => ({
        name: k,
        note: C7_MANY.replace("{key}", k),
        backup: "",
      })),
    ];
    buildNotice({
      role: "status",
      cls: "ok",
      icon: ICONS.check,
      title: "Signed in: your account's settings are loaded",
      body: `${(b.backups || []).length} setting${(b.backups || []).length === 1 ? "" : "s"} in this browser differed from your account.` +
        (b.notBackedUp && b.notBackedUp.length ? " Some copies were not backed up." : ""),
      keys,
    });
  });

  // Offline / too-large notices from the status stream (AC-10, C7).
  let offlineNotice = null;
  let largeNoticed = new Set();
  on("status", (st) => {
    if (!st || st.state !== "offline") {
      if (offlineNotice && st && st.state === "saved") { offlineNotice.remove(); offlineNotice = null; }
      return;
    }
    if (st.reason === "too-large") {
      if (st.key && !largeNoticed.has(st.key)) {
        largeNoticed.add(st.key);
        buildNotice({
          role: "status",
          cls: "warn",
          icon: ICONS.cloudOff,
          title: C7_TOO_LARGE.replace("{key}", st.key),
          body: null,
        });
      }
      return;
    }
    if (!offlineNotice) {
      offlineNotice = buildNotice({
        role: "status",
        cls: "warn",
        icon: ICONS.cloudOff,
        title: "Offline: changes are kept in this browser",
        body: "They upload when the connection returns.",
        actions: [
          { id: "retry", label: "Retry now", run: () => window.VtvSync.retry() },
        ],
      });
    }
  });

  window.addEventListener("online", () => {
    state.online = true;
    for (const [k, s] of slots) {
      if (s.state === "retry" || s.state === "unsynced") { s.attempts = 0; s.state = "dirty"; flush(k); }
    }
  });

  watchAuth();
})();
