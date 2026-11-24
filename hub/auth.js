// TradeX sign-in for the hub header (mockup 01) and the delete-account dialog (mockup 04).
// One `/auth/me` on load; local mode renders nothing. Exposes `window.HubAuth = { me, onChange }`
// for log_panel.js. A credential is never logged or stored anywhere.
(() => {
  const slot = document.getElementById("authSlot");
  if (!slot) return;

  const ONETAP_KEY = "hubOnetapDismissed";   // per tab: a dismissal keeps the prompt away
  const GIS_SRC = "https://accounts.google.com/gsi/client";
  const STATE = { auth: false };             // the last /auth/me payload
  const listeners = new Set();
  let ready = false;
  let menu = null, trigger = null, dialog = null;

  // ---------- state ----------
  const me = fetch("/auth/me", { cache: "no-store" })
    .then((r) => (r.ok ? r.json() : { auth: false }))
    .catch(() => ({ auth: false }))
    .then((s) => {
      Object.assign(STATE, s);
      ready = true;
      render();
      listeners.forEach((fn) => fn(STATE));
      if (STATE.auth) failedNotice();
      return STATE;
    });

  window.HubAuth = {
    me,
    onChange: (fn) => { listeners.add(fn); if (ready) fn(STATE); return () => listeners.delete(fn); },
    current: () => STATE,
  };

  async function reload() {
    const next = await fetch("/auth/me", { cache: "no-store" }).then((r) => (r.ok ? r.json() : { auth: false })).catch(() => ({ auth: false }));
    Object.assign(STATE, next);
    render();
    listeners.forEach((fn) => fn(STATE));
  }

  // ---------- render ----------
  function render() {
    closeMenu(false);
    slot.textContent = "";
    trigger = null;
    if (!STATE.auth) return;                       // local mode: no control at all
    if (STATE.user) renderSignedIn(STATE); else renderSignedOut(STATE);
  }

  const initial = (u) => {
    const src = (u.name && u.name.trim()) || u.email || "?";
    return src.split(/\s+/).filter(Boolean).slice(0, 2).map((w) => w[0].toUpperCase()).join("") || "?";
  };
  const displayName = (u) => (u.name && u.name.trim()) || u.email;

  function renderSignedOut(s) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "auth-btn";
    b.innerHTML = `<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4"/><path d="M10 17l5-5-5-5"/><path d="M15 12H3"/></svg>Sign in with Google`;
    b.onclick = () => {
      // Mockup 01 "signing in": the spinner carries the state, the label stays legible.
      b.innerHTML = `<span class="spin" aria-hidden="true"></span>Signing in`;
      b.setAttribute("aria-busy", "true");
      b.setAttribute("aria-disabled", "true");
      location.href = "/auth/google/start?client=web";
    };
    slot.appendChild(b);
    loadGis(s.client_id);
  }

  function renderSignedIn(s) {
    const u = s.user;
    trigger = document.createElement("button");
    trigger.type = "button";
    trigger.className = "avatar";
    trigger.textContent = initial(u);
    trigger.setAttribute("aria-haspopup", "menu");
    trigger.setAttribute("aria-expanded", "false");
    trigger.setAttribute("aria-controls", "authMenu");
    trigger.setAttribute("aria-label", `Account menu for ${displayName(u)}`);
    trigger.addEventListener("click", () => (menu && isOpen(menu) ? closeMenu(true) : openMenu()));
    trigger.addEventListener("keydown", (e) => {
      if (e.key === "ArrowDown") { e.preventDefault(); openMenu(true); }
    });
    slot.appendChild(trigger);

    menu = document.createElement("div");
    menu.id = "authMenu";
    menu.className = "auth-menu";
    menu.setAttribute("role", "menu");
    menu.setAttribute("aria-label", "Account");
    if (hasPopover()) menu.setAttribute("popover", "auto");
    else menu.hidden = true;

    const who = document.createElement("div");
    who.className = "who";
    who.setAttribute("role", "presentation");
    const av = document.createElement("span");
    av.className = "avatar";
    av.setAttribute("aria-hidden", "true");
    av.textContent = initial(u);
    const box = document.createElement("div");
    const nm = document.createElement("b");
    nm.textContent = displayName(u);
    if (s.admin) { const bd = document.createElement("span"); bd.className = "badge"; bd.textContent = "Admin"; nm.appendChild(bd); }
    const em = document.createElement("span");
    em.textContent = u.email || "";
    box.append(nm, em);
    who.append(av, box);
    menu.appendChild(who);
    menu.appendChild(sep());

    if (s.admin) menu.appendChild(item("logs", "Log panel", "admin"));
    menu.appendChild(item("delete", "Delete account", null, "bad"));
    menu.appendChild(sep());
    menu.appendChild(item("signout", "Sign out"));
    slot.appendChild(menu);

    menu.addEventListener("keydown", onMenuKeys);
    if (!hasPopover()) document.addEventListener("pointerdown", onOutside, true);
    menu.querySelectorAll("[data-act]").forEach((el) => (el.onclick = () => act(el.dataset.act)));
  }

  const ICON = {
    logs: '<path d="M4 17l6-6-6-6"/><path d="M12 19h8"/>',
    trash: '<path d="M3 6h18"/><path d="M8 6V4h8v2"/><path d="M6 6l1 14h10l1-14"/>',
    out: '<path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/><path d="M16 17l5-5-5-5"/><path d="M21 12H9"/>',
  };
  const icon = (d) => `<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${d}</svg>`;

  function item(act, label, sub, cls) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "item" + (cls ? " " + cls : "");
    b.setAttribute("role", "menuitem");
    b.dataset.act = act;
    b.innerHTML = icon(ICON[act === "signout" ? "out" : act === "delete" ? "trash" : "logs"]) + `<span>${label}</span>`;
    if (sub) { const s = document.createElement("span"); s.className = "sub"; s.textContent = sub; b.appendChild(s); }
    return b;
  }
  function sep() { return document.createElement("hr"); }

  // ---------- menu open/close ----------
  const hasPopover = () => "showPopover" in HTMLElement.prototype;
  const isOpen = (m) => (m.hasAttribute("popover") ? m.matches(":popover-open") : !m.hidden);

  function openMenu(fromKeyboard) {
    if (!menu) return;
    if (fromKeyboard) menu.dataset.enter = "1";
    if (hasPopover() && !isOpen(menu)) menu.showPopover();
    else if (!hasPopover()) menu.hidden = false;
    if (trigger) trigger.setAttribute("aria-expanded", "true");
    place();
    if (fromKeyboard) {
      const first = menu.querySelector('[data-act]:not(:disabled)');
      if (first) first.focus();
    }
  }

  function closeMenu(focusTrigger) {
    if (!menu) return;
    if (isOpen(menu)) { if (hasPopover() && menu.hasAttribute("popover")) menu.hidePopover(); else menu.hidden = true; }
    if (trigger) {
      trigger.setAttribute("aria-expanded", "false");
      if (focusTrigger) trigger.focus();
    }
  }

  function place() {
    if (!menu || !trigger) return;
    const r = trigger.getBoundingClientRect(), w = menu.offsetWidth, h = menu.offsetHeight;
    const left = Math.max(8, Math.min(r.right - w, innerWidth - w - 8));
    const below = r.bottom + 6, above = r.top - h - 6;
    menu.style.left = `${left}px`;
    menu.style.top = `${below + h > innerHeight - 8 && above > 8 ? above : below}px`;
  }

  function onMenuKeys(e) {
    const acts = [...menu.querySelectorAll("[data-act]:not(:disabled)")];
    if (!acts.length) return;
    const i = acts.indexOf(document.activeElement);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const n = e.key === "ArrowDown" ? (i + 1) % acts.length : (i <= 0 ? acts.length - 1 : i - 1);
      acts[n].focus();
    } else if (e.key === "Home") { e.preventDefault(); acts[0].focus(); }
    else if (e.key === "End") { e.preventDefault(); acts[acts.length - 1].focus(); }
    else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); closeMenu(true); }
  }

  function onOutside(e) {
    if (menu && !menu.hidden && !menu.contains(e.target) && e.target !== trigger && !trigger?.contains(e.target)) closeMenu(false);
  }

  // ---------- actions ----------
  async function act(name) {
    closeMenu(false);
    if (name === "logs") {
      if (typeof window.openPanelByName === "function") window.openPanelByName("logs");
    } else if (name === "delete") {
      openDelete();
    } else if (name === "signout") {
      try { await fetch("/auth/logout", { method: "POST", headers: { "Content-Type": "application/json" } }); } catch (_) {}
      sessionStorage.removeItem(ONETAP_KEY);
      await reload();
    }
  }

  // ---------- One Tap (signed out only) ----------
  function loadGis(clientId) {
    if (!clientId) return;
    const g = window.google && window.google.accounts && window.google.accounts.id;
    if (g) return initOneTap(clientId);
    if (document.getElementById("authGis")) return;      // already loading
    const s = document.createElement("script");
    s.id = "authGis";
    s.async = true;
    s.defer = true;
    s.src = GIS_SRC;
    // No integrity: Google serves this URL mutable in place (modern-web-guidance
    // "secure-external-subresources" 1: omit SRI on a mutable third-party script).
    s.onload = () => initOneTap(clientId);
    s.onerror = () => {};                                // no prompt; the button flow still works
    document.head.appendChild(s);
  }

  function initOneTap(clientId) {
    if (sessionStorage.getItem(ONETAP_KEY) === "1") return;
    const g = window.google && window.google.accounts && window.google.accounts.id;
    if (!g) return;
    g.initialize({
      client_id: clientId,
      auto_select: false,
      cancel_on_tap_outside: true,
      callback: (r) => { if (r && r.credential) postCredential(r.credential); },
    });
    g.prompt((n) => {
      if (n && typeof n.isDismissedMoment === "function" && n.isDismissedMoment()) {
        sessionStorage.setItem(ONETAP_KEY, "1");
      }
    });
  }

  async function postCredential(credential) {
    try {
      const r = await fetch("/auth/google/onetap", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ credential }),
      });
      if (r.ok) await reload();
    } catch (_) { /* the button flow stays available */ }
  }

  // ---------- delete account (mockup 04) ----------
  function buildDialog() {
    const d = document.createElement("dialog");
    d.id = "authDel";
    d.setAttribute("aria-labelledby", "authDelTitle");
    d.innerHTML = `
      <h3 id="authDelTitle">Delete your TradeX account?</h3>
      <form method="dialog">
        <p class="lead">This deletes, from the server: your account and the Google sign-in link, every
          signed-in session on all devices, your saved layouts, drawings, scripts, favourites and theme,
          and your MT5 bridges.</p>
        <p class="small"><b class="warn">This cannot be undone.</b> What is stored in this browser stays
          here, and you can keep using TradeX signed out.</p>
        <p class="err" role="alert" hidden></p>
        <div class="acts">
          <button type="submit" class="auth-btn" value="cancel" autofocus>Cancel</button>
          <button type="button" class="auth-btn danger" data-go><svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 6h18"/><path d="M8 6V4h8v2"/><path d="M6 6l1 14h10l1-14"/></svg><span>Delete account</span></button>
        </div>
      </form>`;
    d.querySelector("[data-go]").onclick = () => deleteAccount(d);
    d.addEventListener("close", () => { d.returnValue = ""; });
    return d;
  }

  function openDelete() {
    if (!dialog) { dialog = buildDialog(); document.body.appendChild(dialog); }
    const err = dialog.querySelector(".err");
    err.hidden = true;
    resetButtons(dialog);
    dialog.showModal();
  }

  function resetButtons(d) {
    d.removeAttribute("aria-busy");
    const cancel = d.querySelector('button[value="cancel"]');
    const go = d.querySelector("[data-go]");
    cancel.disabled = false;
    go.disabled = false;
    go.removeAttribute("aria-busy");
    go.innerHTML = `<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 6h18"/><path d="M8 6V4h8v2"/><path d="M6 6l1 14h10l1-14"/></svg><span>Delete account</span>`;
  }

  async function deleteAccount(d) {
    const err = d.querySelector(".err");
    const cancel = d.querySelector('button[value="cancel"]');
    const go = d.querySelector("[data-go]");
    err.hidden = true;
    d.setAttribute("aria-busy", "true");
    cancel.disabled = true;
    go.disabled = true;
    go.setAttribute("aria-busy", "true");
    go.innerHTML = `<span class="spin" aria-hidden="true"></span><span>Deleting</span>`;
    let failure = "";
    try {
      const r = await fetch("/api/me", { method: "DELETE" });
      if (r.status === 204) {
        sessionStorage.removeItem(ONETAP_KEY);
        d.close();
        await reload();
        return;
      }
      failure = r.status === 503
        ? "Error: the server is not available right now. Your account is not deleted and you are still signed in. Try again in a moment."
        : "Error: the server refused the request. Your account is not deleted and you are still signed in.";
    } catch (_) {
      failure = "Error: the server did not answer. Your account is not deleted and you are still signed in. Check your connection, then try again.";
    }
    d.removeAttribute("aria-busy");
    resetButtons(d);
    go.innerHTML = `<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 12a9 9 0 1 0 3-6.7"/><path d="M3 4v5h5"/></svg><span>Try again</span>`;
    err.textContent = failure;
    err.hidden = false;
    cancel.focus();
  }

  // ---------- ?signin=failed (mockup 01, frame 3) ----------
  function failedNotice() {
    const p = new URLSearchParams(location.search);
    if (p.get("signin") !== "failed") return;
    history.replaceState(null, "", location.pathname + location.hash);
    if (document.getElementById("authNotice")) return;
    const n = document.createElement("div");
    n.id = "authNotice";
    n.className = "notice";
    n.setAttribute("role", "alert");
    n.innerHTML = `
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 9v4"/><path d="M12 17h.01"/><path d="M10.3 3.9 2.4 18a2 2 0 0 0 1.7 3h15.8a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z"/></svg>
      <div><h3>Error: sign-in did not finish</h3>
        <p>Google did not confirm the account. Your charts and drawings in this browser are unchanged.
          Try again, or keep working signed out.</p></div>
      <div class="acts"><button type="button" class="auth-btn" data-again>Try again</button>
        <button type="button" class="auth-btn plain" data-dismiss>Dismiss</button></div>`;
    n.querySelector("[data-again]").onclick = () => { location.href = "/auth/google/start?client=web"; };
    n.querySelector("[data-dismiss]").onclick = () => n.remove();
    document.body.appendChild(n);
  }
})();
