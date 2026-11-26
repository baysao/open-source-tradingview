// The panel's whole script. No framework, no build step: the window loads ui/index.html directly
// (tauri.conf.json's frontendDist), so this file is the shipped artifact.
//
// The one boundary rule here: every privileged action is an `invoke` the ACL gates. This script
// never builds a command string, never writes a secret to storage, and drops the revoke code as
// soon as the operator has had one chance to copy it.
//
// `window.__TAURI_INTERNALS__` is present even with withGlobalTauri: false (crates/desktop/src/
// init.js:5 records the same fact): the flag controls only the friendly namespace, and the
// capability is what actually gates a call.
//
// Served outside the app (a plain http server, which is how the design gates measure this page,
// DEBT-016's rule) there is no internals object. The panel then says so in one line and disables
// its controls instead of throwing a console error into the render measurement.
const hasApp = typeof window.__TAURI_INTERNALS__ === "object" && window.__TAURI_INTERNALS__ !== null;

const invoke = (cmd, args) => {
  if (!hasApp) {
    return Promise.reject({
      cause: "This page is open outside the app, so no command can run.",
      next_action: "Start BridgeManager; the panel is served by the app itself.",
    });
  }
  return window.__TAURI_INTERNALS__.invoke(cmd, args);
};

// Theme: the hub's two presets, chosen by `?theme=light` (the panel defaults to graphite, the hub's
// own default). A query parameter rather than localStorage so the design gates can measure both
// themes against the same served file — a gate that cannot reach the light theme cannot clear it.
{
  const wanted = new URLSearchParams(location.search).get("theme");
  if (wanted === "light" || wanted === "dark") {
    document.documentElement.dataset.theme = wanted;
  }
}

// How often the panel may re-check on its own. A local panel, not a monitor: slower than 15 s.
const REFRESH_MS = 15000;

const $ = (id) => document.getElementById(id);

function setState(el, text, cls) {
  el.textContent = text;
  el.classList.remove("ok", "bad", "unknown");
  if (cls) el.classList.add(cls);
}

/** Renders a failure as cause plus next action, both verbatim (CUJ 7). */
function showError(el, err) {
  el.replaceChildren();
  const cause = document.createElement("span");
  cause.className = "bad";
  cause.textContent = err && err.cause ? err.cause : String(err);
  el.appendChild(cause);
  if (err && err.next_action) {
    const next = document.createElement("span");
    next.className = "next";
    next.textContent = err.next_action;
    el.appendChild(next);
  }
}

function showLine(el, text) {
  el.replaceChildren();
  const span = document.createElement("span");
  span.textContent = text;
  el.appendChild(span);
}

/** Runs one privileged call with the busy state every button needs (R-26: no dead controls). */
async function withBusy(button, fn) {
  const label = button.textContent;
  button.disabled = true;
  button.textContent = "Working…";
  try {
    return await fn();
  } finally {
    button.disabled = false;
    button.textContent = label;
  }
}

// ---------------------------------------------------------------- status

let lastStatusAt = 0;

function applyStatus(s) {
  setState(
    $("st-local"),
    s.local_detail,
    s.local_ok ? "ok" : "bad",
  );
  // The task lines carry their own name and state ("VTVBridge: running"); unknown is not "stopped".
  setState($("st-bridge-task"), s.bridge_task, s.bridge_task.includes("running") ? "ok"
    : s.bridge_task.includes("unknown") ? "unknown" : "bad");
  setState($("st-tunnel-task"), s.tunnel_task, s.tunnel_task.includes("running") ? "ok"
    : s.tunnel_task.includes("unknown") ? "unknown" : "bad");
  setState($("st-edge"), s.edge_detail, s.edge_ok ? "ok" : "unknown");
  lastStatusAt = Date.now();
  const secs = Math.max(0, Math.round(s.elapsed_ms / 100) / 10);
  $("status-when").textContent = `checked ${new Date().toLocaleTimeString()} in ${secs}s`;
}

async function refreshStatus(button) {
  const run = async () => {
    try {
      const s = await invoke("status", { bridgePort: null });
      applyStatus(s);
      showLine($("status-result"), "");
    } catch (e) {
      showError($("status-result"), e);
    }
  };
  if (button) return withBusy(button, run);
  return run();
}

// ---------------------------------------------------------------- pair

function openPair() {
  $("pair-dialog-result").replaceChildren();
  const dlg = $("pair-dialog");
  if (typeof dlg.showModal === "function") dlg.showModal();
  $("f-code").focus();
}

async function submitPair(event) {
  event.preventDefault();
  const button = $("pair-submit");
  const result = $("pair-dialog-result");
  await withBusy(button, async () => {
    try {
      const view = await invoke("pair", {
        hubUrl: $("f-hub").value,
        label: $("f-label").value,
        bridgeUrl: $("f-bridge").value,
        token: $("f-token").value,
        code: $("f-code").value,
      });
      $("pair-dialog").close();
      setState($("pair-state"), `${view.label} — ${view.symbol_count} symbols`, "ok");
      // The revoke code gets exactly one screen. Copy is a gesture; nothing is stored.
      const box = document.createElement("div");
      box.className = "secret";
      const code = document.createElement("code");
      code.textContent = view.revoke_code;
      const copy = document.createElement("button");
      copy.type = "button";
      copy.textContent = "Copy";
      copy.addEventListener("click", async () => {
        try {
          await navigator.clipboard.writeText(view.revoke_code);
          copy.textContent = "Copied";
        } catch {
          copy.textContent = "Select and copy";
        }
      });
      const note = document.createElement("span");
      note.className = "note";
      note.textContent = "Revoke code — shown once. Keep it: it is the only way to unregister from here.";
      box.append(code, copy, note);
      const host = $("pair-result");
      host.replaceChildren(box);
      // Clear the credential fields from the DOM: a form left filled is a secret left lying about.
      $("f-token").value = "";
      $("f-code").value = "";
      refreshStatus(null);
    } catch (e) {
      showError(result, e);
    }
  });
}

// ---------------------------------------------------------------- control

async function control(button) {
  const svc = button.dataset.svc;
  const act = button.dataset.act;
  await withBusy(button, async () => {
    try {
      const line = await invoke("control", { svc, action: act });
      showLine($("control-result"), line);
      // The state the panel shows is now stale: re-read rather than assume.
      await refreshStatus(null);
    } catch (e) {
      showError($("control-result"), e);
    }
  });
}

// ---------------------------------------------------------------- install

// ---------------------------------------------------------------- install

// The install polls its own progress: the webview has no event bus (AC-2 forbids core:event:*), so
// the backend cannot push and this loop pulls. One timer, cleared on every exit path.
let progressTimer = null;

function paintProgress(p) {
  const box = $("install-progress");
  if (!p || !p.total) return;
  box.hidden = false;
  const pct = Math.round((Math.min(p.index, p.total) / p.total) * 100);
  $("prog-fill").style.width = `${pct}%`;
  $("install-progress").querySelector(".bar").setAttribute("aria-valuenow", String(p.index));
  const label = p.index > 0 ? `${p.index} of ${p.total}: ${p.label}` : p.label;
  // Only touch the DOM when the text changed: a per-poll rewrite would make aria-live chatter.
  if ($("prog-label").textContent !== label) $("prog-label").textContent = label;
  if ($("prog-detail").textContent !== (p.detail || "")) $("prog-detail").textContent = p.detail || "";
}

function stopProgressPolling() {
  if (progressTimer !== null) {
    clearInterval(progressTimer);
    progressTimer = null;
  }
}

function startProgressPolling() {
  stopProgressPolling();
  const tick = async () => {
    try {
      const p = await invoke("install_progress", {});
      paintProgress(p);
      if (p && p.done) stopProgressPolling();
    } catch {
      // The app is closing or the invoke was refused; stop rather than spin against a dead router.
      stopProgressPolling();
    }
  };
  tick();
  progressTimer = setInterval(tick, 400);
}

async function install(button) {
  await withBusy(button, async () => {
    const host = $("install-result");
    $("install-progress").hidden = false;
    paintProgress({ index: 0, total: 8, label: "starting", detail: "preparing", done: false });
    startProgressPolling();
    try {
      const out = await invoke("install", {
        deployRoot: null,
        mcpToken: $("mcp-token").value || null,
        mcpUrl: $("mcp-url").value || null,
      });
      $("mcp-token").value = "";
      const list = document.createElement("ul");
      list.style.margin = "6px 0 0";
      list.style.paddingLeft = "18px";
      for (const step of out.steps) {
        const li = document.createElement("li");
        const outcome = typeof step.outcome === "string" ? step.outcome : Object.keys(step.outcome)[0];
        li.textContent = `${step.name}: ${outcome}`;
        if (step.outcome && step.outcome.Skipped) {
          li.className = "unknown";
          li.textContent = `${step.name}: skipped — ${step.outcome.Skipped}`;
        }
        // A measurement, not a failure: the endpoint answered, and the status is the proof.
        if (step.outcome && step.outcome.Observed) {
          li.className = "ok";
          li.textContent = `${step.name}: ${step.outcome.Observed}`;
        }
        list.appendChild(li);
      }
      host.replaceChildren(list);
      if (!out.tunnel_ready) {
        const warn = document.createElement("span");
        warn.className = "next";
        warn.textContent = "The tunnel is not registered yet: run `cloudflared tunnel login` in a browser on this host, then install again.";
        host.appendChild(warn);
      }
      await refreshStatus(null);
    } catch (e) {
      showError(host, e);
    } finally {
      // Read the final snapshot before stopping, so a fast run does not leave the bar at 0.
      try {
        paintProgress(await invoke("install_progress", {}));
      } catch { /* the error path already told the operator what happened */ }
      stopProgressPolling();
    }
  });
}

// The MCP address and token are checked AUTOMATICALLY: 1.2 s after the last keystroke in either
// field, and once on load. No button — the dots on the two labels are the whole control. One
// command, two verdicts (the ACL stays one entry wide): the address probe carries no credential, the
// token probe carries it, so a stale token can never masquerade as a wrong port.
const MCP_CHECK_DEBOUNCE_MS = 1200;
let mcpCheckTimer = null;

function paintMcpProbe(id, state, words) {
  const dot = $(id);
  dot.classList.remove("ok", "bad");
  if (state) dot.classList.add(state);
  const label = `${id === "probe-url" ? "address" : "token"}: ${words}`;
  dot.setAttribute("aria-label", label);
  dot.setAttribute("title", label);
}

async function runMcpCheck() {
  const host = $("mcp-result");
  try {
    const out = await invoke("check_mcp", {
      mcpUrl: $("mcp-url").value || null,
      mcpToken: $("mcp-token").value || null,
    });
    paintMcpProbe("probe-url", out.url_ok ? "ok" : "bad", out.url_ok ? "reachable" : "not reachable");
    paintMcpProbe(
      "probe-token",
      out.token_ok === null ? null : out.token_ok ? "ok" : "bad",
      out.token_ok === null ? "not checked" : out.token_ok ? "valid" : "refused",
    );
    setState(host, out.token_detail, out.token_ok === false ? "bad" : out.token_ok ? "ok" : "unknown");
  } catch {
    // Preview mode or a refusing router: dots stay neutral, the fields stay editable.
    paintMcpProbe("probe-url", null, "not checked");
    paintMcpProbe("probe-token", null, "not checked");
  }
}

function scheduleMcpCheck() {
  clearTimeout(mcpCheckTimer);
  mcpCheckTimer = setTimeout(runMcpCheck, MCP_CHECK_DEBOUNCE_MS);
}

// ---------------------------------------------------------------- revoke

function openRevoke() {
  const dlg = $("revoke-dialog");
  if (typeof dlg.showModal === "function") dlg.showModal();
}

async function confirmRevoke() {
  $("revoke-dialog").close();
  const button = $("revoke-run");
  await withBusy(button, async () => {
    try {
      const line = await invoke("revoke", { hubUrl: null });
      setState($("pair-state"), "not paired", "unknown");
      $("pair-result").replaceChildren();
      showLine($("revoke-result"), line);
    } catch (e) {
      showError($("revoke-result"), e);
    }
  });
}

// ---------------------------------------------------------------- wiring

$("pair-open").addEventListener("click", openPair);
$("pair-form").addEventListener("submit", submitPair);
$("pair-cancel").addEventListener("click", () => $("pair-dialog").close());
$("status-refresh").addEventListener("click", (e) => refreshStatus(e.currentTarget));
for (const b of document.querySelectorAll("button[data-svc]")) {
  b.addEventListener("click", () => control(b));
}
$("install-run").addEventListener("click", (e) => install(e.currentTarget));
for (const id of ["mcp-url", "mcp-token"]) {
  $(id).addEventListener("input", scheduleMcpCheck);
}
// One check at load so the address dot is green before the operator types anything.
if (hasApp) runMcpCheck();
$("revoke-run").addEventListener("click", openRevoke);
$("revoke-confirm").addEventListener("click", confirmRevoke);
$("revoke-cancel").addEventListener("click", () => $("revoke-dialog").close());

// The panel's own timer: never faster than REFRESH_MS, and it yields while a dialog is open so a
// re-render cannot fight the operator's typing.
setInterval(() => {
  if (document.querySelector("dialog[open]")) return;
  if (Date.now() - lastStatusAt < REFRESH_MS) return;
  refreshStatus(null);
}, REFRESH_MS);

// One check at load, so the panel is never a wall of "not checked yet". Outside the app there is
// nothing to check: the panel says why, in one line, and leaves the controls live but honest (each
// one names the same cause when pressed).
if (!hasApp) {
  const note = document.createElement("p");
  note.className = "note";
  note.textContent =
    "Preview mode: this page is open outside BridgeManager, so no command can run. Start the app to use it.";
  $("status-when").replaceWith(note);
} else {
  refreshStatus(null);
}
