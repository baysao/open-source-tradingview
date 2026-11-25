// 04-02 served-page check (AC-1 browser level, AC-3 end to end, AC-5 reconnect).
// Run from the repo root, after starting the servers it expects:
//   node .paul/runs/check-0402.mjs
// Servers (the wrapper .paul/runs/check-0402.sh starts and stops them):
//   hosted: hub-server --features test-idp, AUTH_ENABLED=true, PUBLIC_ORIGIN=http://127.0.0.1:8771,
//           VTV_TEST_IDP=1, pgsqlite over a temp file  -> HOSTED
//   local:  hub-server (no auth)                        -> LOCAL (optional, --local-only)
// L-012: all state lives on the page object, never globalThis.
// PW_INDEX_MJS overrides the playwright module (CI: PW_INDEX_MJS=playwright with the
// package installed in the repo root; the default is the ux-ui kit copy on the dev box).
const { chromium } = await import(process.env.PW_INDEX_MJS ||
  "file:///C:/Users/Dell/.agents/ux-ui-agent-skills/node_modules/playwright/index.mjs");
import fs from "node:fs";
import path from "node:path";

const HOSTED = process.env.CHECK_HOSTED || "http://127.0.0.1:8771";
const LOCAL = process.env.CHECK_LOCAL || "";
const OUT = path.resolve(".paul/runs/shots-0402");
fs.mkdirSync(OUT, { recursive: true });
const results = [];
const log = (ok, name, detail = "") => {
  results.push({ ok, name, detail });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? " :: " + detail : ""}`);
};

// The GIS script never reaches Google: answer it with a stub that keeps the callback.
const GIS_STUB = `
window.google = { accounts: { id: {
  initialize(cfg) { window.__cfg = cfg; },
  prompt(cb) { window.__promptCb = cb; },
  cancel() {},
} } };
`;

const PAGES = ["hub.html", "vela.html", "lightweight.html", "klinechart.html"];

/** A page whose console errors are recorded, with the beacon POSTs it made. */
async function newPage(browser, { width = 1440, height = 900 } = {}) {
  const ctx = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: 1 });
  const page = await ctx.newPage();
  await page.route("https://accounts.google.com/gsi/client", (route) =>
    route.fulfill({ status: 200, contentType: "text/javascript", body: GIS_STUB })
  );
  const seen = { console: [], csp: [], ttfc: [], frameTtfc: [], ttfcResponses: [] };
  page.on("console", (m) => {
    const t = m.text();
    seen.console.push(`${m.type()}: ${t}`);
    if (/Content Security Policy|Refused to/i.test(t)) seen.csp.push(t);
  });
  page.on("pageerror", (e) => seen.console.push(`pageerror: ${String(e)}`));
  page.on("request", (r) => {
    const u = new URL(r.url());
    if (u.pathname !== "/api/metrics/ttfc") return;
    // hub.html frames vela.html, and a frame's requests reach the page listener too: only the
    // top-level document counts here, or the wrapper looks like it sent the beacon itself.
    if (r.frame() !== page.mainFrame()) {
      seen.frameTtfc.push(r.url());
      return;
    }
    let body = null;
    try { body = r.postDataJSON(); } catch (_) { body = r.postData(); }
    seen.ttfc.push({ method: r.method(), body, headers: r.headers() });
  });
  page.on("response", async (r) => {
    const u = new URL(r.url());
    if (u.pathname !== "/api/metrics/ttfc") return;
    seen.ttfcResponses.push(r.status());
  });
  page.__seen = seen;
  return page;
}

const shots = (page, name) => page.screenshot({ path: path.join(OUT, `${name}.png`), fullPage: false });

/** Waits until the page (or its frame) has sent `n` beacons; true when it got there in time.
 *
 * The beacon fires on the first candle, and this feed delivers one 8-9 s after load, so any fixed
 * sleep is a guess about the network. Polling the recorded requests is the real signal.
 */
async function waitBeacons(page, { frame = false, n = 1, timeoutMs = 45000 } = {}) {
  const list = () => (frame ? page.__seen.frameTtfc : page.__seen.ttfc);
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (list().length >= n) return true;
    await page.waitForTimeout(250);
  }
  return false;
}

async function signInViaMint(page, { sub = "g-0402", email } = {}) {
  const body = { sub };
  if (email) body.email = email;
  const r = await page.request.post(`${HOSTED}/__test-idp/mint`, { data: body });
  const { credential } = await r.json();
  const res = await page.request.post(`${HOSTED}/auth/google/onetap`, {
    data: { credential },
    headers: { Origin: HOSTED, "Content-Type": "application/json" },
  });
  return res.status();
}

// PW_CHANNEL overrides the launch channel (CI: chromium via `npx playwright install`;
// the dev box uses the system Chrome - the kit build's own chromium download is absent).
const browser = await chromium.launch({ channel: process.env.PW_CHANNEL || "chrome" });

// ---------- (a0) the served header is the page's own policy, at the wire
// AC-1 asks for the response header to equal `CspSet::scan`'s policy for that path. The browser
// proves the policy is workable; this proves it is the right one, and it is the only place
// index.html is measured (it redirects instantly to hub.html, so the page loop skips it).
// The unit test in tests/gate.rs pins the same shape against a fixture root; this one runs
// against the pages that actually ship.
{
  const fetchHeader = async (p) => {
    const r = await fetch(`${HOSTED}${p}`, { redirect: "manual" });
    return { status: r.status, csp: r.headers.get("content-security-policy") || "" };
  };
  const pages = ["hub.html", "vela.html", "lightweight.html", "klinechart.html", "index.html"];
  const policies = {};
  for (const p of pages) {
    const { status, csp } = await fetchHeader(`/${p}`);
    policies[p] = csp;
    log(status === 200 && csp.length > 0, `AC-1 ${p}: served with a CSP`, `status=${status} len=${csp.length}`);
  }
  // hub.html is the only page that loads auth.js, so it is the only one naming the GIS origin.
  log(/accounts\.google\.com/.test(policies["hub.html"]), "AC-1 hub.html: policy names the GIS origin");
  for (const p of pages.filter((x) => x !== "hub.html")) {
    log(!/accounts\.google\.com/.test(policies[p]), `AC-1 ${p}: no GIS origin in its policy`);
  }
  // index.html has no inline block: its header is the strict default, not a page policy.
  log(
    /default-src 'none'/.test(policies["index.html"]) && !/sha256-/.test(policies["index.html"]),
    "AC-1 index.html: carries the default policy (no per-page hashes)",
    policies["index.html"].slice(0, 60)
  );
  // Every engine page hashes its own inline block, and the four hashes differ (each page's block
  // is its own): a shared policy would be one hash repeated.
  const hashes = pages
    .filter((p) => p !== "index.html")
    .map((p) => (policies[p].match(/sha256-[A-Za-z0-9+/=]+/g) || [])[0] || "");
  log(hashes.every(Boolean), "AC-1 every script page names a sha256 hash", hashes.join(" "));
  log(new Set(hashes).size === hashes.length, "AC-1 the four page hashes are all different", `distinct=${new Set(hashes).size}`);
  // An API response carries the strict default, never a page policy.
  const api = await fetch(`${HOSTED}/api/health`);
  const apiCsp = api.headers.get("content-security-policy") || "";
  log(apiCsp === policies["index.html"], "AC-1 /api/health carries the strict default", apiCsp.slice(0, 60));
  // A missing page still carries a policy: never an unprotected body.
  const missing = await fetchHeader("/nope-0402.html");
  log(missing.status === 404 && missing.csp.length > 0, "AC-1 a 404 still carries a CSP", `status=${missing.status}`);
}

// ---------- (a) every page loads with zero CSP violations
for (const file of PAGES) {
  const page = await newPage(browser);
  const failures = [];
  page.on("requestfailed", (r) => failures.push(`${r.url()} ${r.failure()?.errorText}`));
  await page.goto(`${HOSTED}/${file}`, { waitUntil: "networkidle" });
  await page.waitForTimeout(1500);
  const csp = page.__seen.csp;
  log(csp.length === 0, `AC-1 ${file}: no CSP violation in the console`, csp.slice(0, 2).join(" | "));
  // A blocked subresource (the usual CSP casualty) also counts, even when Chrome logs only once.
  const domainBlocked = failures.filter((f) => /\/(theme|lab|auth|ttfc|vela\.bundle|pine_|ind_|orderflow_|drawing_favbar|log_panel)\.js|fonts\.googleapis/.test(f));
  log(domainBlocked.length === 0, `AC-1 ${file}: no blocked page script or font`, domainBlocked.slice(0, 3).join(" | "));
  if (file === "hub.html") await shots(page, "01-hub");
  await page.context().close();
}

// ---------- (b) sign-in flow through the test-idp mint renders a signed-in state
{
  const page = await newPage(browser);
  await page.goto(`${HOSTED}/hub.html`, { waitUntil: "networkidle" });
  const st = await signInViaMint(page, { sub: "g-0402", email: "owner@example.test" });
  log(st === 200, "AC-1 sign-in through the test-idp mint", String(st));
  await page.reload({ waitUntil: "networkidle" });
  const avatar = page.locator("#authSlot > .avatar");
  log((await avatar.count()) === 1, "AC-1 signed-in state renders (avatar)");
  await page.context().close();
}

// ---------- (c) a Vela chart draws bars, and (d) the page posts exactly one beacon
{
  const page = await newPage(browser);
  const beaconOnLoad = page.__seen.ttfc;
  await page.goto(`${HOSTED}/vela.html`, { waitUntil: "networkidle" });
  // L-007: real UI, real log. The vela page sets canvas dimensions once bars arrive.
  const drew = await page.waitForFunction(() => {
    const c = document.querySelector("#chart canvas");
    return !!c && c.width > 100 && c.height > 100;
  }, null, { timeout: 30000 }).then(() => true, () => false);
  log(drew, "AC-1 vela.html: a chart canvas is drawn");
  // Live bars arrive on the feed's clock, not on load: wait for the beacon itself.
  await waitBeacons(page, { n: 1 });
  await page.waitForTimeout(500);   // a second beacon would land in the same window
  const beacons = page.__seen.ttfc;
  log(beacons.length === 1, "AC-3 vela.html: exactly one beacon POST", `count=${beacons.length}`);
  if (beacons.length) {
    const b = beacons[0];
    const okShape = b.method === "POST" && b.body && typeof b.body.ms === "number" && b.body.engine === "vela";
    log(okShape, "AC-3 vela.html: beacon body is {ms, engine: 'vela'}", JSON.stringify(b.body));
    log(b.body && b.body.ms >= 0 && b.body.ms <= 120000, "AC-3 vela.html: ms is in range", String(b.body && b.body.ms));
    log(String(b.headers["content-type"]).includes("application/json"), "AC-3 vela.html: beacon is JSON", String(b.headers["content-type"]));
  }
  const statuses = page.__seen.ttfcResponses;
  log(statuses.length > 0 && statuses.every((s) => s === 204), "AC-3 vela.html: the beacon was accepted (204)", JSON.stringify(statuses));
  log(page.__seen.csp.length === 0, "AC-1 vela.html: no CSP violation while drawing", page.__seen.csp.slice(0, 2).join(" | "));
  // (c2) The Pine worker: this is the only code on any page that constructs a blob: Worker, so it
  // is the only thing that makes `worker-src 'self' blob:` load-bearing. Vela runs each script in
  // its own worker (vela.bundle.js: URL.createObjectURL(new Blob(...)) -> new Worker(url)), so with
  // the directive gone the constructor is blocked, the script never runs, and the console carries
  // the violation. Assert the run, not just the absence of errors.
  await shots(page, "02-vela");
  const pine = await page.evaluate(async () => {
    const source = '//@version=6\nindicator("0402 probe", overlay = true)\nplot(ta.sma(close, 5), color = color.orange)\n';
    const res = await window.labChart.addPine(source);
    // `ready` means the worker is up; the first run report arrives on its own event after it, so
    // poll instead of reading once (a synchronous read here returned null on a healthy script).
    let run = null;
    const deadline = Date.now() + 20000;
    while (Date.now() < deadline) {
      run = res && res.id ? window.labChart.lastRun(res.id) : null;
      if (run && Object.keys(run.plots || {}).length > 0) break;
      await new Promise((r) => setTimeout(r, 200));
    }
    return { res, run };
  });
  log(!!(pine.res && pine.res.ok), "AC-1 vela.html: a Pine indicator loads", JSON.stringify(pine.res));
  const plots = pine.run && pine.run.plots ? Object.keys(pine.run.plots).length : 0;
  log(plots > 0, "AC-1 vela.html: the Pine script produced plot values", JSON.stringify(pine.run));
  log(page.__seen.csp.length === 0, "AC-1 vela.html: no CSP violation while running Pine", page.__seen.csp.slice(0, 2).join(" | "));
  await shots(page, "05-pine");
  await page.context().close();
  void beaconOnLoad;
}

// ---------- (d) the other two engine pages send their own one beacon; hub.html sends none
for (const [file, engine] of [["lightweight.html", "lightweight"], ["klinechart.html", "klinechart"], ["hub.html", null]]) {
  const page = await newPage(browser);
  await page.goto(`${HOSTED}/${file}`, { waitUntil: "networkidle" });
  // hub.html frames vela: the beacon arrives from the frame, never from the wrapper.
  const arrived = await waitBeacons(page, { frame: !engine, n: 1 });
  // A wrapper that does send one would already be in ttfc, so give that a beat too.
  if (engine) await page.waitForTimeout(1500);
  const beacons = page.__seen.ttfc;
  if (engine) {
    log(arrived && beacons.length === 1, `AC-3 ${file}: exactly one beacon POST`, `arrived=${arrived} count=${beacons.length}`);
    log(beacons[0] && beacons[0].body && beacons[0].body.engine === engine, `AC-3 ${file}: the beacon names ${engine}`, JSON.stringify(beacons[0] && beacons[0].body));
    await shots(page, `03-${engine}`);
  } else {
    log(arrived && beacons.length === 0, "AC-3 hub.html: no beacon from the wrapper page", `frame=${page.__seen.frameTtfc.length} wrapper=${beacons.length}`);
    // The framed chart must still be instrumented: the hub is a shell, not a chart.
    log(arrived, "AC-3 hub.html: the framed vela page sent its own beacon", `count=${page.__seen.frameTtfc.length}`);
  }
  log(page.__seen.csp.length === 0, `AC-1 ${file}: no CSP violation`, page.__seen.csp.slice(0, 2).join(" | "));
  await page.context().close();
}

// ---------- (e) GET /admin/metrics as admin returns the shape with count >= 1
{
  const page = await newPage(browser);
  await page.goto(`${HOSTED}/hub.html`, { waitUntil: "networkidle" });
  const anon = await page.evaluate(async () => (await fetch("/admin/metrics", { cache: "no-store" })).status);
  log(anon === 404, "AC-3 /admin/metrics signed out is 404", String(anon));
  await signInViaMint(page, { sub: "g-admin", email: "owner@example.test" });
  const read = await page.evaluate(async () => {
    const r = await fetch("/admin/metrics", { cache: "no-store" });
    return { status: r.status, body: await r.json() };
  });
  log(read.status === 200, "AC-3 /admin/metrics as admin is 200", String(read.status));
  // GET /admin/metrics returns the ttfc view directly: {since, engines}.
  const t = read.body;
  log(!!t && typeof t.since === "number", "AC-3 read carries since", JSON.stringify(t && t.since));
  const now = Date.now();
  log(!!t && t.since > 0 && t.since <= now, "AC-3 since is a past unix-ms instant", `since=${t && t.since} now=${now}`);
  const since = t && t.since;
  log(!!t && typeof since === "number" && since > 0 && since <= now && since > now - 30 * 60 * 1000,
    "AC-3 since is the process start, not boot time or zero", `since=${since} now=${now}`);
  const engines = t && t.engines ? Object.keys(t.engines) : [];
  const anyCount = engines.some((e) => t.engines[e].count >= 1);
  log(anyCount, "AC-3 the histogram recorded the beacons this check sent", JSON.stringify(t && t.engines));
  const shapesOk = engines.every((e) => {
    const r = t.engines[e];
    return typeof r.count === "number" && typeof r.p50_ms === "number" && typeof r.p75_ms === "number" && typeof r.p95_ms === "number"
      && r.p50_ms <= r.p75_ms && r.p75_ms <= r.p95_ms;
  });
  log(shapesOk, "AC-3 every engine row is {count, p50_ms, p75_ms, p95_ms} and monotone", JSON.stringify(t && t.engines));
  await page.context().close();
}

// ---------- (g) reconnect: kill and restart the server between two bar windows (AC-5)
// Driven by the wrapper: it restarts the server while this block waits, then reports through a file.
const RECONNECT = process.env.CHECK_RECONNECT === "1";
if (RECONNECT) {
  const page = await newPage(browser);
  const huds = [];
  page.on("console", (m) => { if (/Feed offline|retrying/i.test(m.text())) huds.push(m.text()); });
  await page.goto(`${HOSTED}/vela.html`, { waitUntil: "networkidle" });
  await page.waitForFunction(() => {
    const c = document.querySelector("#chart canvas");
    return !!c && c.width > 100;
  }, null, { timeout: 30000 });
  // A baseline of zero proves nothing: wait until real bars have flowed, then kill the server.
  const flowed = await page.waitForFunction(() => window.labStats.updates >= 2, null, { timeout: 60000 })
    .then(() => true, () => false);
  log(flowed, "AC-5 bars were flowing before the server was killed");
  // timeOrigin is fixed for the life of the document: if it changes, the page navigated.
  const before = await page.evaluate(() => ({ updates: window.labStats.updates, timeOrigin: performance.timeOrigin }));
  fs.writeFileSync(path.resolve(".paul/runs/shots-0402/reconnect-go"), "go");
  const offline = await page.waitForFunction(
    () => /feed offline, retrying in/i.test(document.getElementById("lab-hud")?.textContent || ""),
    null, { timeout: 60000 }
  ).then(() => true, () => false);
  log(offline, "AC-5 lab.js shows the reconnect countdown after the socket closed");
  if (offline) await shots(page, "04-reconnect-countdown");
  // Bars must resume without a reload: the stats counter only moves on real bars.
  const resumed = await page.waitForFunction(
    (n) => window.labStats.updates > n,
    before.updates, { timeout: 90000 }
  ).then(() => true, () => false);
  const after = await page.evaluate(() => ({ updates: window.labStats.updates, timeOrigin: performance.timeOrigin }));
  log(resumed, "AC-5 bars resume without a page reload", `${before.updates} -> ${after.updates}`);
  log(after.timeOrigin === before.timeOrigin, "AC-5 the same document served the resumed bars (timeOrigin unchanged)",
    `${before.timeOrigin} vs ${after.timeOrigin}`);
  fs.writeFileSync(path.resolve(".paul/runs/shots-0402/reconnect-done"), JSON.stringify({ before, after }));
  await page.context().close();
}

// ---------- (f) local mode: the same pages under the same CSP, plus one beacon and no sign-in
if (LOCAL) {
  const page = await newPage(browser);
  await page.goto(`${LOCAL}/hub.html`, { waitUntil: "networkidle" });
  await page.waitForTimeout(1200);
  log(page.__seen.csp.length === 0, "AC-1 local hub.html: no CSP violation", page.__seen.csp.slice(0, 2).join(" | "));
  log((await page.locator("#authSlot").innerHTML()) === "", "AC-1 local mode: no sign-in control");
  await page.context().close();

  const vela = await newPage(browser);
  await vela.goto(`${LOCAL}/vela.html`, { waitUntil: "networkidle" });
  const drew = await vela.waitForFunction(() => {
    const c = document.querySelector("#chart canvas");
    return !!c && c.width > 100;
  }, null, { timeout: 30000 }).then(() => true, () => false);
  log(drew, "AC-1 local vela.html: a chart canvas is drawn");
  await waitBeacons(vela, { n: 1 });
  await vela.waitForTimeout(500);
  log(vela.__seen.ttfc.length === 1, "AC-3 local vela.html: exactly one beacon POST", `count=${vela.__seen.ttfc.length}`);
  log(vela.__seen.csp.length === 0, "AC-1 local vela.html: no CSP violation", vela.__seen.csp.slice(0, 2).join(" | "));
  await vela.context().close();
}

await browser.close();

const failed = results.filter((r) => !r.ok);
fs.writeFileSync(path.join(OUT, "results.json"), JSON.stringify({ hosted: HOSTED, local: LOCAL || null, passed: results.length - failed.length, failed: failed.length, results }, null, 2));
console.log(`\n${results.length - failed.length}/${results.length} passed`);
if (failed.length) {
  console.log("FAILURES:");
  failed.forEach((f) => console.log(` - ${f.name} :: ${f.detail}`));
  process.exit(1);
}
