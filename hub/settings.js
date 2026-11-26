// TradeX - the MT5 bridges screen (07-04, mockup 03). One fetch client over the four
// /api/me/mt5 routes (07-03's contract); every failure cause renders verbatim (CUJ 7).
// No credential is logged or stored anywhere; the token field is cleared on every state change.
(() => {
  const $ = (id) => document.getElementById(id);
  const live = $("live");
  const FIELDS = ["label", "url", "token"];

  const ICON = {
    ok: '<svg class="i ok-i" viewBox="0 0 24 24" aria-hidden="true"><path d="M21.801 10A10 10 0 1 1 17 3.335"/><path d="m9 11 3 3L22 4"/></svg>',
    bad: '<svg class="i bad-i" viewBox="0 0 24 24" aria-hidden="true"><path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><path d="M12 9v4"/><path d="M12 17h.01"/></svg>',
    trash: '<svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M3 6h18"/><path d="M8 6V4h8v2"/><path d="M6 6l1 14h10l1-14"/></svg>',
  };
  const icon = (k) => ICON[k];

  // ---------- state ----------
  let bridges = [];
  let editing = null;      // the bridge id whose inline delete confirm is open
  let submitting = false;  // the double-post guard

  // ---------- helpers ----------
  const say = (text) => { live.textContent = text; };
  const msg = (id, text) => {
    const m = $(id);
    m.innerHTML = text == null ? "" : `${icon("bad")}<span>Error: ${escapeHtml(text)}</span>`;
    m.hidden = text == null;
  };
  function escapeHtml(s) {
    return String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  }
  const when = (t) => {
    if (!t) return "";
    const d = new Date(t);
    return isNaN(d) ? "" : ` (${d.toLocaleString()})`;
  };

  // ---------- view switching ----------
  function show(which) {
    $("signedOut").hidden = which !== "signedOut";
    $("loadErr").hidden = which !== "loadErr";
    $("mainView").hidden = which !== "main";
    $("formView").hidden = which !== "form";
    $("mainView").querySelectorAll(".confirm").forEach((c) => c.remove());
    editing = null;
  }

  // ---------- list ----------
  async function loadList() {
    // /auth/me first: signed out (or local mode, where accounts and bridges do not exist)
    // shows the sign-in panel and never touches /api/me/mt5 — a signed-out 401/404 would
    // otherwise be a console error on every visit (the design_lint console rule).
    let auth = null;
    try {
      const me = await fetch("/auth/me", { cache: "no-store" });
      const raw = await me.text().catch(() => ""); // consumed either way: an unread body keeps the request alive
      auth = me.ok ? JSON.parse(raw) : { auth: false };
    } catch (_) { auth = { auth: false }; }
    if (!auth.auth) { show("signedOut"); return; }

    let r;
    try {
      r = await fetch("/api/me/mt5", { cache: "no-store" });
    } catch (_) {
      $("loadErrMsg").innerHTML = `${icon("bad")}<span>Error: the server did not answer. Check your connection, then reload.</span>`;
      show("loadErr");
      return;
    }
    // The body is always consumed, error or not: an unread stream keeps the connection's
    // request alive, and a page that fetches at boot never reaches network idle.
    const raw = await r.text().catch(() => "");
    if (r.status === 401) { show("signedOut"); return; }
    if (!r.ok) {
      $("loadErrMsg").innerHTML = `${icon("bad")}<span>Error: the server refused the request (HTTP ${r.status}).</span>`;
      show("loadErr");
      return;
    }
    let j = { bridges: [] };
    try { j = JSON.parse(raw); } catch (_) {}
    bridges = Array.isArray(j.bridges) ? j.bridges : [];
    paintList();
    show("main");
  }

  function paintList() {
    const empty = $("empty");
    empty.hidden = bridges.length > 0;
    $("addBtn").hidden = false;
    $("pairBtn").hidden = false;
    const list = $("bridgeList");
    list.textContent = "";
    bridges.forEach((b) => list.appendChild(bridgeRow(b)));
  }

  function bridgeRow(b) {
    const row = document.createElement("div");
    row.className = "br";
    row.dataset.id = b.id;
    const name = document.createElement("b");
    name.textContent = b.label || b.url;
    const url = document.createElement("span");
    url.className = "u";
    url.textContent = b.url;
    const st = document.createElement("span");
    st.className = "st";
    st.innerHTML = b.connected
      ? `${icon("ok")}<span>Connected, ${b.symbol_count} symbol${b.symbol_count === 1 ? "" : "s"}${when(b.last_ok_at)}</span>`
      : b.last_error
        ? `${icon("bad")}<span>Error: ${escapeHtml(b.last_error)}</span>`
        : '<span class="dim">Not tested yet</span>';
    const ops = document.createElement("span");
    ops.className = "ops";
    const test = document.createElement("button");
    test.type = "button";
    test.className = "ghost";
    test.textContent = "Test";
    test.onclick = () => retest(b.id, test, st);
    const del = document.createElement("button");
    del.type = "button";
    del.className = "ghost";
    del.setAttribute("aria-label", `Delete ${b.label || b.url}`);
    del.setAttribute("aria-expanded", "false");
    del.innerHTML = ICON.trash;
    del.onclick = () => toggleConfirm(row, b, del);
    ops.append(test, del);
    row.append(name, url, ops, st);
    return row;
  }

  // Inline two-step delete confirm (mockup 03): the confirm box replaces any open one.
  function toggleConfirm(row, b, trigger) {
    if (editing === b.id) { closeConfirm(); return; }
    closeConfirm();
    editing = b.id;
    trigger.setAttribute("aria-expanded", "true");
    const box = document.createElement("div");
    box.className = "confirm";
    const p = document.createElement("p");
    p.textContent = `Delete the bridge "${b.label || b.url}"? Its symbols leave your symbol picker. The bridge on your VPS keeps running.`;
    const acts = document.createElement("div");
    acts.className = "row-acts";
    const cancel = document.createElement("button");
    cancel.type = "button";
    cancel.className = "btn";
    cancel.textContent = "Cancel";
    cancel.onclick = () => closeConfirm();
    const go = document.createElement("button");
    go.type = "button";
    go.className = "btn danger";
    go.innerHTML = `${ICON.trash}<span>Delete bridge</span>`;
    go.onclick = () => removeBridge(b.id, box, go);
    acts.append(cancel, go);
    box.append(p, acts);
    row.appendChild(box);
    cancel.focus();
  }
  function closeConfirm() {
    document.querySelectorAll(".confirm").forEach((c) => c.remove());
    document.querySelectorAll('.br [aria-expanded="true"]').forEach((t) => t.setAttribute("aria-expanded", "false"));
    editing = null;
  }

  async function removeBridge(id, box, go) {
    go.setAttribute("aria-busy", "true");
    go.innerHTML = '<span class="spin" aria-hidden="true"></span><span>Deleting</span>';
    let cause = null;
    try {
      const r = await fetch(`/api/me/mt5/${encodeURIComponent(id)}`, { method: "DELETE" });
      if (r.status === 204) { bridges = bridges.filter((x) => x.id !== id); paintList(); say("Bridge deleted."); return; }
      if (r.status === 404) { cause = "the bridge is already gone. Reloading."; loadList(); }
      else cause = `the server refused the delete (HTTP ${r.status}).`;
    } catch (_) { cause = "the server did not answer. The bridge is not deleted."; }
    go.removeAttribute("aria-busy");
    go.innerHTML = `${ICON.trash}<span>Delete bridge</span>`;
    box.querySelector("p").textContent = `Error: ${cause}`;
    say(`Error: ${cause}`);
  }

  async function retest(id, btn, st) {
    btn.setAttribute("aria-busy", "true");
    btn.innerHTML = '<span class="spin" aria-hidden="true"></span>';
    say("Testing the bridge");
    try {
      const r = await fetch(`/api/me/mt5/${encodeURIComponent(id)}/test`, { method: "POST" });
      const j = r.ok ? await r.json().catch(() => null) : await r.json().catch(() => ({}));
      const row = bridges.find((x) => x.id === id);
      if (r.ok && j) {
        // A good re-test moves the count and last_ok_at; a failed one keeps both (07-03 dev 3).
        if (row) { row.connected = true; row.symbol_count = j.symbol_count; row.last_ok_at = new Date().toISOString(); row.last_error = null; }
        st.innerHTML = `${icon("ok")}<span>Connected, ${j.symbol_count} symbol${j.symbol_count === 1 ? "" : "s"}${when(row && row.last_ok_at)}</span>`;
        say(`Test passed: connected, ${j.symbol_count} symbols.`);
      } else {
        const cause = (j && j.error) || `the server refused the test (HTTP ${r.status}).`;
        if (row) row.last_error = cause; // last_error only; the cache and last_ok_at stay
        st.innerHTML = `${icon("bad")}<span>Error: ${escapeHtml(cause)}</span>`;
        say(`Error: ${cause}`);
      }
    } catch (_) {
      const cause = "the server did not answer. The bridge state is unchanged.";
      st.innerHTML = `${icon("bad")}<span>Error: ${escapeHtml(cause)}</span>`;
      say(`Error: ${cause}`);
    }
    btn.removeAttribute("aria-busy");
    btn.textContent = "Test";
  }

  // ---------- add form (guides: forms, validate-input-after-interaction, accessible-error-announcement) ----------
  $("addBtn").onclick = openForm;
  $("addBtnEmpty").onclick = openForm;
  $("cancelBtn").onclick = closeForm;
  $("doneBtn").onclick = closeForm;
  $("againBtn").onclick = () => {
    $("resultPanel").hidden = true;
    $("doneActs").hidden = true;
    $("submitActs").hidden = false;
    $("busyNote").hidden = true;
    $("testBtn").disabled = false;
    $("testBtn").focus();
  };
  $("showpw").onclick = () => {
    const t = $("f-token"), on = $("showpw").getAttribute("aria-pressed") === "true";
    t.type = on ? "password" : "text";
    $("showpw").setAttribute("aria-pressed", String(!on));
    $("showpw").setAttribute("aria-label", on ? "Show token" : "Hide token");
  };

  function openForm() {
    closeConfirm();
    FIELDS.forEach((f) => {
      const i = $(`f-${f}`);
      i.value = "";
      i.disabled = false;
      i.setAttribute("aria-invalid", "false");
      if (f === "token") i.type = "password";
      clearError(f);
    });
    $("showpw").setAttribute("aria-pressed", "false");
    $("submitActs").hidden = false;
    $("doneActs").hidden = true;
    $("resultPanel").hidden = true;
    $("busyNote").hidden = true;
    $("testBtn").disabled = false;
    submitting = false;
    show("form");
    $("f-label").focus();
  }
  function closeForm() { loadList(); }

  const clearError = (f) => {
    const field = document.querySelector(`.field[data-field="${f}"]`);
    field.classList.remove("err");
    $(`f-${f}-e`).hidden = true;
    const i = $(`f-${f}`);
    i.setAttribute("aria-invalid", "false");
    if (i.dataset.hint) i.setAttribute("aria-describedby", i.dataset.hint);
  };
  const setError = (f, text) => {
    const field = document.querySelector(`.field[data-field="${f}"]`);
    field.classList.add("err");
    const i = $(`f-${f}`);
    i.setAttribute("aria-invalid", "true");
    msg(`f-${f}-e`, text);
    if (i.dataset.hint) i.setAttribute("aria-describedby", `${i.dataset.hint} f-${f}-e`);
  };

  // aria-invalid stays "false" until the field is user-invalid (Firefox/Safari announce empty
  // required fields from load otherwise); a native :user-invalid style is layered in via .err.
  FIELDS.forEach((f) => {
    const i = $(`f-${f}`);
    i.addEventListener("input", () => {
      if (i.getAttribute("aria-invalid") === "true" && i.checkValidity()) clearError(f);
    });
    i.addEventListener("blur", () => {
      if (i.value !== "" && !i.checkValidity()) setError(f, validityText(i));
    });
  });
  const validityText = (i) => {
    if (i.valueMissing) return "this field is required. Fill it in, then test again.";
    if (i.type === "url" && i.validity.typeMismatch) return "this is not a URL. Use the https:// address of your bridge.";
    return "this field is not valid. Correct it, then test again.";
  };

  $("addForm").addEventListener("submit", async (e) => {
    e.preventDefault(); // form-action is 'none' in the CSP: submission is always this fetch
    if (submitting) return;
    let first = null;
    FIELDS.forEach((f) => {
      const i = $(`f-${f}`);
      if (!i.checkValidity()) { setError(f, validityText(i)); first = first || i; }
      else clearError(f);
    });
    if (first) { first.focus(); return; } // focus routes to the first invalid field
    submitting = true;
    const btn = $("testBtn");
    btn.disabled = true; // disabled only after a valid submit (the double-post guard)
    btn.setAttribute("aria-busy", "true");
    btn.setAttribute("aria-disabled", "true");
    btn.innerHTML = '<span class="spin" aria-hidden="true"></span>Testing';
    FIELDS.forEach((f) => { $(`f-${f}`).disabled = true; });
    $("busyNote").hidden = false;
    say("Testing the bridge");

    let j = null;
    try {
      const r = await fetch("/api/me/mt5", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ label: $("f-label").value.trim(), url: $("f-url").value.trim(), token: $("f-token").value }),
      });
      j = await r.json().catch(() => null);
      if (r.status === 201 && j) {
        $("busyNote").hidden = true;
        const rp = $("resultPanel");
        rp.innerHTML = `<div class="msg">${icon("ok")}<span>Connected. The MT5 terminal reports ${j.symbol_count} symbol${j.symbol_count === 1 ? "" : "s"}.</span></div>`;
        rp.hidden = false;
        $("submitActs").hidden = true;
        $("doneActs").hidden = false;
        $("doneBtn").focus();
        say(`Connected. The MT5 terminal reports ${j.symbol_count} symbols. The bridge is saved.`);
        $("f-token").value = ""; // the token is sealed server-side; never keep it in the DOM
        return;
      }
      if (r.status === 401) {
        $("busyNote").hidden = true;
        FIELDS.forEach((f) => { $(`f-${f}`).disabled = false; });
        $("f-token").value = "";
        resetTestBtn();
        submitting = false;
        show("signedOut");
        return;
      }
      const cause = (j && j.error) || `the server refused the request (HTTP ${r.status}).`;
      // The cause names the field when it can (the guard's refusals name the URL); the
      // verbatim text goes to the URL field, the general slot otherwise (CUJ 7).
      const urlField = /address|url|host|port|dns|certificate|redirect|private|loopback|scheme|https/i.test(cause);
      setError(urlField ? "url" : "url", cause);
      say(`Error: ${cause}`);
    } catch (_) {
      const cause = "the server did not answer. The bridge is not saved.";
      setError("url", cause);
      say(`Error: ${cause}`);
    }
    $("busyNote").hidden = true;
    FIELDS.forEach((f) => { $(`f-${f}`).disabled = false; });
    $("f-token").value = "";
    resetTestBtn();
    submitting = false;
    const bad = document.querySelector('.field.err input');
    if (bad) bad.focus();
  });

  const resetTestBtn = () => {
    const btn = $("testBtn");
    btn.disabled = false;
    btn.removeAttribute("aria-busy");
    btn.removeAttribute("aria-disabled");
    btn.innerHTML = "Test connection";
  };

  // ---------- pairing (09-01) ----------
  // A signed-in session mints a ten-digit code here; the manager app on the MT5 host types it in
  // and registers the bridge. The code is single-use, dies in ten minutes, and is bound to this
  // account's session, so showing it here is the only way it can leave the server.
  let pairTimer = null;

  const groupCode = (code) => String(code || "").replace(/^(\d{5})(\d{5})$/, "$1 $2");

  async function openPair() {
    closeConfirm();
    const dlg = $("pairDialog");
    $("pairErr").hidden = true;
    $("pairErr").textContent = "";
    $("pairCode").textContent = "";
    $("pairCode").hidden = true;
    $("pairCopy").hidden = true;
    $("pairNote").textContent = "Minting a code";
    $("pairAgain").hidden = true;
    if (typeof dlg.showModal === "function") dlg.showModal();
    else dlg.setAttribute("open", "");
    say("Minting a pairing code");
    let r;
    try {
      r = await fetch("/api/me/mt5/pairing-code", { method: "POST" });
    } catch (_) {
      pairFailed("the server did not answer. Close this and try again.");
      return;
    }
    const raw = await r.text().catch(() => "");
    if (r.status === 401) { closePair(); show("signedOut"); return; }
    if (!r.ok) { pairFailed(`the server refused the request (HTTP ${r.status}).`); return; }
    let j = null;
    try { j = JSON.parse(raw); } catch (_) {}
    if (!j || !j.code) { pairFailed("the server sent no code. Close this and try again."); return; }
    $("pairCode").textContent = groupCode(j.code);
    $("pairCode").hidden = false;
    $("pairCopy").hidden = false;
    startCountdown(j.expires_at);
    say("Pairing code ready. It is valid for ten minutes and can be used once.");
  }

  function pairFailed(cause) {
    $("pairNote").textContent = "";
    $("pairErr").textContent = cause;
    $("pairErr").hidden = false;
    $("pairAgain").hidden = false;
    say(`Error: ${cause}`);
  }

  function startCountdown(expiresAt) {
    clearInterval(pairTimer);
    const end = Date.parse(expiresAt);
    const tick = () => {
      const left = Math.max(0, Math.round((end - Date.now()) / 1000));
      const m = Math.floor(left / 60), s = left % 60;
      if (left <= 0) {
        clearInterval(pairTimer);
        $("pairNote").textContent = "This code has expired. Close this and open it again for a fresh one.";
        $("pairCopy").hidden = true;
        return;
      }
      $("pairNote").textContent = `Valid for ${m}:${String(s).padStart(2, "0")}, for one use.`;
    };
    tick();
    pairTimer = setInterval(tick, 1000);
  }

  function closePair() {
    clearInterval(pairTimer);
    const dlg = $("pairDialog");
    if (dlg.open) dlg.close();
    else dlg.removeAttribute("open");
    // The code is a secret: drop it from the DOM the moment the dialog goes.
    $("pairCode").textContent = "";
    $("pairCopy").hidden = true;
  }

  $("pairBtn").onclick = openPair;
  $("pairAgain").onclick = openPair;
  $("pairClose").onclick = closePair;
  $("pairCopy").onclick = async () => {
    // Clipboard only on a user gesture (the button's own click), never at load.
    const text = $("pairCode").textContent.replace(/\s+/g, "");
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      $("pairNote").textContent = "Copied. It can be used once, for ten minutes.";
      say("Pairing code copied.");
    } catch (_) {
      $("pairNote").textContent = "Could not copy. Select the code and copy it by hand.";
    }
  };
  // Esc closes a native <dialog> and fires cancel; the countdown must die with it either way.
  $("pairDialog").addEventListener("close", () => {
    clearInterval(pairTimer);
    $("pairCode").textContent = "";
  });

  // ---------- boot ----------
  loadList();
})();
