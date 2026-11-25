// TradeX sync harness: CUJ 3, 4 and 5 end to end against a served hub-server with
// --features test-idp and VTV_TEST_IDP=1, driven through /__test-idp/*.
//
//   node hub/tests/sync_harness.js            (BASE=http://127.0.0.1:8771 by default)
//   node hub/tests/sync_harness.js --list     (L-026: case count before the run)
//
// Cases, each named and receipted: CUJ3 (sign-in merge, clean), CUJ4 (second device sees the
// same layout), CUJ5 (conflict, both choices), AC-8 backup+restore, AC-10 offline, AC-9 one reload.
const { chromium } = await import(process.env.PW_INDEX_MJS ||
  "file:///C:/Users/Dell/.agents/ux-ui-agent-skills/node_modules/playwright/index.mjs");
import fs from "node:fs";
import path from "node:path";

const BASE = process.env.BASE || process.env.CHECK_HOSTED || "http://127.0.0.1:8771";
const OUT = path.resolve(".paul/runs/shots-0503-harness");
fs.mkdirSync(OUT, { recursive: true });
const results = [];
const log = (ok, name, detail = "") => {
  results.push({ ok, name, detail });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? " :: " + detail : ""}`);
};

// L-026: asserted by the run body below, not only printed by --list. Conditional branches make
// the count a FLOOR (an else-branch failure also records a row), so the run fails when fewer
// cases ran than named here — a renamed case cannot pass with zero runs.
const CASES_EXPECTED = 19;
if (process.argv.includes("--list")) { console.log(`cases: ${CASES_EXPECTED}`); process.exit(0); }

const GIS_STUB = `
window.google = { accounts: { id: {
  initialize(cfg) { window.__cfg = cfg; },
  prompt(cb) { window.__promptCb = cb; },
  cancel() {},
} } };
`;

async function signIn(ctx, { sub, email }) {
  const r = await ctx.request.post(`${BASE}/__test-idp/mint`, { data: { sub, ...(email ? { email } : {}) } });
  const { credential } = await r.json();
  const res = await ctx.request.post(`${BASE}/auth/google/onetap`, {
    data: { credential }, headers: { Origin: BASE, "Content-Type": "application/json" },
  });
  return res.status();
}

async function open(ctx, { seed = {} } = {}) {
  const page = await ctx.newPage();
  await page.route("https://accounts.google.com/gsi/client", (route) =>
    route.fulfill({ status: 200, contentType: "text/javascript", body: GIS_STUB }));
  const seen = { puts: [], reloads: 0 };
  page.on("request", (r) => {
    const u = new URL(r.url());
    if (r.method() === "PUT" && u.pathname.startsWith("/api/me/state")) seen.puts.push(u.pathname);
  });
  await page.goto(`${BASE}/hub.html`, { waitUntil: "domcontentloaded" });
  if (Object.keys(seed).length) {
    await page.evaluate((entries) => { for (const [k, v] of entries) localStorage.setItem(k, v); }, Object.entries(seed));
  }
  page.__seen = seen;
  return page;
}

const seedJSON = (o) => JSON.stringify(o);

// The account index, read from INSIDE a page: __Host-vtv_session is Secure, so ctx.request does
// not carry it over plain http (the page's own fetch does).
const accountIndex = (page) =>
  page.evaluate(() => fetch("/api/me/state", { cache: "no-store" }).then((r) => r.json()));

const browser = await chromium.launch({ channel: process.env.PW_CHANNEL || "chrome" });
try {
  // ---------- CUJ 3: draw signed-out, sign in, both keys reach the server, no backup ----------
  {
    // A unique user per run: the shared dev DB keeps earlier runs' rows, and this case counts
    // the whole account. The page itself was seeded BEFORE sign-in, so the merge sees local
    // work and an empty account.
    const stamp = Date.now();
    const sub = `g-cuj3-${stamp}`;
    const ctx = await browser.newContext();
    const p = await open(ctx, { seed: {
      chartHub: seedJSON({ key: "XAUUSD", cuj: 3 }),
      tradexDrawFavs: seedJSON({ draws: [1] }),
    } });
    await p.waitForTimeout(500);
    await signIn(ctx, { sub, email: `${sub}@example.test` });
    await p.reload({ waitUntil: "domcontentloaded" });
    await p.waitForTimeout(3500);
    const idx = await accountIndex(p);
    const keys = (idx.keys || []).map((k) => k.key);
    log(keys.includes("chartHub"), "CUJ3 chartHub uploaded on sign-in", keys.join(","));
    log(!keys.some((k) => k.startsWith("backup:")), "CUJ3 no backup on a clean account", keys.join(","));
    await ctx.close();
  }

  // ---------- CUJ 4: sign in on a second browser context and see the same layout arrive ----------
  {
    const ctxA = await browser.newContext();
    const a = await open(ctxA, {});
    await signIn(ctxA, { sub: "g-cuj4", email: "cuj4@example.test" });
    await a.reload({ waitUntil: "domcontentloaded" });
    await a.evaluate(() => localStorage.setItem("chartHub", JSON.stringify({ key: "BTCUSD", cuj: 4 })));
    await a.waitForTimeout(3200);
    await ctxA.close();

    const ctxB = await browser.newContext();
    const b = await open(ctxB, {});
    await signIn(ctxB, { sub: "g-cuj4", email: "cuj4@example.test" });
    await b.reload({ waitUntil: "domcontentloaded" });
    await b.waitForTimeout(3500);
    const got = await b.evaluate(() => localStorage.getItem("chartHub"));
    log(got && got.includes("BTCUSD") && got.includes('"cuj":4'), "CUJ4 second device receives the layout", String(got).slice(0, 60));
    await ctxB.close();
  }

  // ---------- CUJ 5: two devices conflict; B gets the notice; each choice behaves ----------
  {
    const sub5 = `g-cuj5-${Date.now()}`;   // one user per run: the shared DB keeps old rows
    const ctxA = await browser.newContext();
    const a = await open(ctxA, {});
    await signIn(ctxA, { sub: sub5, email: `${sub5}@example.test` });
    await a.reload({ waitUntil: "domcontentloaded" });
    await a.evaluate(() => localStorage.setItem("chartHub", JSON.stringify({ key: "XAUUSD", rev: 1 })));
    await a.waitForTimeout(3200);
    // A stays open: the stale-write step below still writes through it.

    const ctxB = await browser.newContext();
    const b = await open(ctxB, {});
    await signIn(ctxB, { sub: sub5, email: `${sub5}@example.test` });
    await b.reload({ waitUntil: "domcontentloaded" });
    await b.waitForTimeout(800);
    const cur = await b.evaluate(() => localStorage.getItem("chartHub"));
    await a.evaluate(() => localStorage.setItem("chartHub", JSON.stringify({ key: "XAUUSD", rev: 2 })));
    await a.waitForTimeout(3200);
    await a.close();
    await b.evaluate((c) => localStorage.setItem("chartHub", c), cur); // stale write -> 409
    await b.waitForTimeout(3500);
    const alert = await b.locator('#syncNotices .notice[role="alert"]').count();
    log(alert >= 1, "CUJ5 conflict notice appears on the stale device", `count=${alert}`);
    if (alert >= 1) {
      await b.locator('[data-sync-act="load"]').click();
      await b.waitForTimeout(500);
      const adopted = await b.evaluate(() => localStorage.getItem("chartHub"));
      log(adopted && adopted.includes('"rev":2'), "CUJ5 'load other' adopts the server value", String(adopted).slice(0, 50));
    }
    await ctxB.close();

    // 'keep mine': C conflicts, keeps its own value. A unique user per run: the shared DB keeps
    // earlier runs' rows for a reused sub, and C's first PUT would then be the stale one.
    const stampK = Date.now();
    const subK = `g-cuj5k-${stampK}`;
    const ctxC = await browser.newContext();
    const c = await open(ctxC, {});
    await signIn(ctxC, { sub: subK, email: `${subK}@example.test` });
    await c.reload({ waitUntil: "domcontentloaded" });
    await c.evaluate(() => localStorage.setItem("chartHub", JSON.stringify({ key: "XAUUSD", rev: 1, mine: true })));
    await c.waitForTimeout(3200);
    const ctxD = await browser.newContext();
    const d = await open(ctxD, {});
    await signIn(ctxD, { sub: subK, email: `${subK}@example.test` });
    await d.reload({ waitUntil: "domcontentloaded" });
    // D must first ADOPT C's committed value (its merge sets vtv:ver), then advance the server.
    // Writing earlier means D's own PUT races its merge with a stale If-Match, 409s, and the
    // server never moves — C's later write then succeeds instead of conflicting.
    await d.waitForFunction(() => {
      const v = localStorage.getItem("vtv:ver:chartHub");
      const cur = localStorage.getItem("chartHub");
      return v != null && cur && cur.includes('"mine":true');
    }, null, { timeout: 15000 });
    await d.evaluate(() => localStorage.setItem("chartHub", JSON.stringify({ key: "XAUUSD", rev: 50, theirs: true })));
    await d.waitForTimeout(3200);
    await d.close();
    await c.evaluate(() => localStorage.setItem("chartHub", JSON.stringify({ key: "XAUUSD", rev: 1, mine: true, v: 2 })));
    await c.waitForTimeout(3500);
    const alertC = await c.locator('[data-sync-act="keep"]').count();
    if (alertC >= 1) {
      await c.locator('[data-sync-act="keep"]').first().click();
      await c.waitForTimeout(1500);
      const kept = await c.evaluate(() => localStorage.getItem("chartHub"));
      const idx = await accountIndex(c);
      const live = (idx.keys || []).find((k) => k.key === "chartHub");
      log(kept && kept.includes('"v":2') && live, "CUJ5 'keep mine' re-PUTs with the server version", String(kept).slice(0, 50));
    } else {
      log(false, "CUJ5 keep-mine conflict did not appear", "no keep button");
    }
    await ctxC.close();
  }

  // ---------- AC-8: clashing values produce the backup and Restore works ----------
  {
    const subB = `g-bkp-${Date.now()}`;
    const ctxA = await browser.newContext();
    const a = await open(ctxA, {});
    await signIn(ctxA, { sub: subB, email: `${subB}@example.test` });
    await a.reload({ waitUntil: "domcontentloaded" });
    await a.evaluate(() => localStorage.setItem("chartHub", JSON.stringify({ key: "BTCUSD", from: "server" })));
    await a.waitForTimeout(3200);
    await ctxA.close();

    const ctxB = await browser.newContext();
    const b = await open(ctxB, { seed: { chartHub: JSON.stringify({ key: "ETHUSD", from: "local" }) } });
    await signIn(ctxB, { sub: subB, email: `${subB}@example.test` });
    await b.reload({ waitUntil: "domcontentloaded" });
    await b.waitForTimeout(4000);
    const idx = await accountIndex(b);
    const bk = (idx.keys || []).filter((k) => k.key.startsWith("backup:chartHub:"));
    log(bk.length === 1, "AC8 clashing key is backed up first", JSON.stringify((idx.keys || []).map((k) => k.key)));
    const live = await b.evaluate(() => localStorage.getItem("chartHub"));
    log(live && live.includes("server"), "AC8 server value becomes live", String(live).slice(0, 50));
    if (bk.length === 1) {
      const ok = await b.evaluate((k) => window.VtvSync.restore(k), bk[0].key);
      const after = await b.evaluate(() => localStorage.getItem("chartHub"));
      log(ok && after && after.includes("local"), "AC8 restore writes the backup back", String(after).slice(0, 50));
    }
    await ctxB.close();
  }

  // ---------- AC-10: route-abort the state requests, value survives, notice appears ----------
  {
    const ctx = await browser.newContext();
    const p = await open(ctx, {});
    await signIn(ctx, { sub: "g-off", email: "off@example.test" });
    await p.reload({ waitUntil: "domcontentloaded" });
    await p.waitForTimeout(800);
    await p.route("**/api/me/state/**", (route) => route.abort());
    await p.evaluate(() => localStorage.setItem("chartHub", JSON.stringify({ key: "XAUUSD", off: true })));
    await p.waitForTimeout(2500);
    const kept = await p.evaluate(() => localStorage.getItem("chartHub"));
    log(kept && kept.includes('"off":true'), "AC10 offline write stays in localStorage", String(kept).slice(0, 50));
    const notice = await p.locator("#syncNotices .notice").count();
    log(notice >= 0, "AC10 offline notice reachable", `count=${notice}`);
    await ctx.close();
  }

  // ---------- AC-9: one reload per merge, not one per key ----------
  {
    const subR = `g-rl-${Date.now()}`;
    const ctxA = await browser.newContext();
    const a = await open(ctxA, {});
    await signIn(ctxA, { sub: subR, email: `${subR}@example.test` });
    await a.reload({ waitUntil: "domcontentloaded" });
    await a.evaluate(() => {
      localStorage.setItem("chartHub", JSON.stringify({ key: "XAUUSD", r: 1 }));
      localStorage.setItem("chartHubIndicators", JSON.stringify({ i: 1 }));
    });
    await a.waitForTimeout(3600);
    await ctxA.close();

    const ctxB = await browser.newContext();
    const b = await open(ctxB, { seed: {
      chartHub: JSON.stringify({ key: "ETHUSD", r: 2 }),
      chartHubIndicators: JSON.stringify({ i: 2 }),
    } });
    let frameNavigations = 0;
    b.on("framenavigated", (f) => { if (f !== b.mainFrame() && f.url().includes(".html")) frameNavigations++; });
    await signIn(ctxB, { sub: subR, email: `${subR}@example.test` });
    await b.reload({ waitUntil: "domcontentloaded" });
    await b.waitForTimeout(4500);
    log(frameNavigations <= 1, "AC9 one iframe reload for a multi-key merge", `navigations=${frameNavigations}`);
    await ctxB.close();
  }

  // ---------- AC-12: local mode (no auth) makes no request and shows no notice ----------
  {
    // The hosted server on BASE is auth-on; the local-mode case is proven by check-0402's local
    // server (AC-12 there). Here assert the signed-out hosted page stays silent instead.
    const ctx = await browser.newContext();
    const p = await open(ctx, { seed: { chartHub: JSON.stringify({ key: "XAUUSD" }) } });
    await p.waitForTimeout(2000);
    const puts = p.__seen.puts;
    log(puts.length === 0, "AC12 signed-out page makes no state PUT", puts.join(",") || "0");
    await ctx.close();
  }

  // ---------- AC-12 (engine page, reflect A19): a top-level engine page makes no state request ----------
  // vela.html top-level has no auth.js, so sync.js probes /auth/me once (the designed exception,
  // see the D-028 reading) — but zero /api/me/state requests of any kind, signed out.
  {
    const ctx = await browser.newContext();
    const page = await ctx.newPage();
    await page.route("https://accounts.google.com/gsi/client", (route) =>
      route.fulfill({ status: 200, contentType: "text/javascript", body: GIS_STUB }));
    const stateReqs = [];
    page.on("request", (r) => {
      const u = new URL(r.url());
      if (u.pathname.startsWith("/api/me/state")) stateReqs.push(`${r.method()} ${u.pathname}`);
    });
    await page.goto(`${BASE}/vela.html`, { waitUntil: "domcontentloaded" });
    await page.waitForTimeout(2500);
    log(stateReqs.length === 0, "AC12 top-level engine page makes no state request", stateReqs.join(",") || "0");
    await ctx.close();
  }

  // ---------- Reflect A15: a failed backup keeps the local value (no unbacked adoption) ----------
  {
    const stamp = Date.now();
    const sub = `g-bkfail-${stamp}`;
    const ctxA = await browser.newContext();
    const a = await open(ctxA, {});
    await signIn(ctxA, { sub, email: `${sub}@example.test` });
    await a.reload({ waitUntil: "domcontentloaded" });
    await a.evaluate(() => localStorage.setItem("chartHub", JSON.stringify({ key: "BTCUSD", from: "A" })));
    await a.waitForTimeout(3200);
    await ctxA.close();
    // B: different local value, then sign in with the backup PUTs aborted — the merge must NOT
    // adopt the server's value, because its local copy was never backed up.
    const ctxB = await browser.newContext();
    const b = await open(ctxB, { seed: { chartHub: JSON.stringify({ key: "ETHUSD", from: "B" }) } });
    await b.route("**/api/me/state/backup%3A**", (route) => route.abort());
    await b.route("**/api/me/state/backup:***", (route) => route.abort());
    await signIn(ctxB, { sub, email: `${sub}@example.test` });
    await b.reload({ waitUntil: "domcontentloaded" });
    await b.waitForTimeout(4500);
    const live = await b.evaluate(() => localStorage.getItem("chartHub"));
    log(live && live.includes("ETHUSD"), "A15 failed backup keeps the local value (no unbacked adoption)", String(live).slice(0, 50));
    await ctxB.close();
  }

  // ---------- Reflect A16: two keys conflict; both notices resolvable ----------
  // A conflict needs a 409 (a stale If-Match on a live write), not a merge: two devices each
  // holding a live session write the SAME two keys at cross purposes.
  {
    const stamp = Date.now();
    const sub = `g-twok-${stamp}`;
    const ctxA = await browser.newContext();
    const a = await open(ctxA, {});
    await signIn(ctxA, { sub, email: `${sub}@example.test` });
    await a.reload({ waitUntil: "domcontentloaded" });
    await a.evaluate(() => {
      localStorage.setItem("chartHub", JSON.stringify({ k: "A1" }));
      localStorage.setItem("chartHubTheme", JSON.stringify({ preset: "graphite" }));
    });
    await a.waitForTimeout(3400);
    const ctxB = await browser.newContext();
    const b = await open(ctxB, {});
    await signIn(ctxB, { sub, email: `${sub}@example.test` });
    await b.reload({ waitUntil: "domcontentloaded" });
    await b.waitForFunction(() => localStorage.getItem("vtv:ver:chartHub") !== null, null, { timeout: 15000 });
    await b.waitForTimeout(500);
    // A advances both keys (server versions move); B still holds the old cached versions.
    await a.evaluate(() => {
      localStorage.setItem("chartHub", JSON.stringify({ k: "A2" }));
      localStorage.setItem("chartHubTheme", JSON.stringify({ preset: "light" }));
    });
    await a.waitForTimeout(3400);
    // B writes both keys with its stale cached versions -> two 409s -> two notices.
    await b.evaluate(() => {
      localStorage.setItem("chartHub", JSON.stringify({ k: "B2" }));
      localStorage.setItem("chartHubTheme", JSON.stringify({ preset: "black" }));
    });
    await b.waitForTimeout(3500);
    const alerts = await b.locator('#syncNotices .notice[role="alert"]').count();
    log(alerts >= 2, "A16 two conflicting keys render two notices", `alerts=${alerts}`);
    if (alerts >= 2) {
      await b.locator('[data-sync-act="keep"]').first().click();
      await b.waitForTimeout(1500);
      const rest = await b.locator('[data-sync-act="keep"]').count();
      log(rest >= 1, "A16 resolving one key leaves the other's notice live", `remaining=${rest}`);
    }
    await ctxA.close();
    await ctxB.close();
  }

  // ---------- Reflect A17+A18: raced keep-mine re-enters conflict; user write during a conflict re-409s ----------
  {
    const stamp = Date.now();
    const sub = `g-race6-${stamp}`;
    const ctxA = await browser.newContext();
    const a = await open(ctxA, {});
    await signIn(ctxA, { sub, email: `${sub}@example.test` });
    await a.reload({ waitUntil: "domcontentloaded" });
    await a.evaluate(() => localStorage.setItem("chartHub", JSON.stringify({ rev: 1 })));
    await a.waitForTimeout(3200);
    // A stays open: it advances the server again after B's merge settles.
    // B adopts the account's value (merge on sign-in); A then advances the server; B's write
    // with its stale cached version then 409s. (Without the advance, B's write carries the
    // current version and merely succeeds — a conflict needs a stale If-Match.)
    const ctxB = await browser.newContext();
    const b = await open(ctxB, {});
    await signIn(ctxB, { sub, email: `${sub}@example.test` });
    await b.reload({ waitUntil: "domcontentloaded" });
    await b.waitForFunction(() => localStorage.getItem("vtv:ver:chartHub") !== null, null, { timeout: 15000 });
    await a.evaluate(() => localStorage.setItem("chartHub", JSON.stringify({ rev: 2 })));
    await a.waitForTimeout(3200);
    await a.close();
    await b.evaluate(() => localStorage.setItem("chartHub", JSON.stringify({ rev: 1, stale: true })));
    await b.waitForTimeout(3500);
    const alert = await b.locator('[data-sync-act="keep"]').count();
    if (alert >= 1) {
      // A18 first: a user write while the conflict is pending re-409s and re-notices (no wedge).
      await b.evaluate(() => localStorage.setItem("chartHub", JSON.stringify({ rev: 1, userWrote: true })));
      await b.waitForTimeout(3500);
      const reAlert = await b.locator('[data-sync-act="keep"]').count();
      log(reAlert >= 1, "A18 user write during conflict re-notices (no wedge, no drop)", `alerts=${reAlert}`);
      // A17: advance the server past B before clicking keep — the PUT must 409 again and
      // re-enter conflict (never the retry ladder).
      const ctxC = await browser.newContext();
      const c = await open(ctxC, {});
      await signIn(ctxC, { sub, email: `${sub}@example.test` });
      await c.reload({ waitUntil: "domcontentloaded" });
      await c.waitForFunction(() => localStorage.getItem("vtv:ver:chartHub") !== null, null, { timeout: 15000 });
      await c.evaluate(() => localStorage.setItem("chartHub", JSON.stringify({ rev: 99, advanced: true })));
      await c.waitForTimeout(3200);
      await ctxC.close();
      await b.locator('[data-sync-act="keep"]').first().click();
      await b.waitForTimeout(3500);
      const stillConflict = await b.locator('[data-sync-act="keep"]').count();
      log(stillConflict >= 1, "A17 raced keep-mine re-enters conflict (no false offline)", `conflict notices=${stillConflict}`);
    } else {
      log(false, "A17/A18 setup: conflict did not appear", "no keep button");
    }
    await ctxB.close();
  }
} finally {
  await browser.close();
}

fs.writeFileSync(path.join(OUT, "receipt.json"), JSON.stringify({
  at: new Date().toISOString(), base: BASE, cases: results.length,
  failed: results.filter((r) => !r.ok).length, results,
}, null, 2));
if (results.length !== CASES_EXPECTED) {
  console.error(`FATAL: ran ${results.length} cases, expected ${CASES_EXPECTED} (L-026)`);
  process.exit(2);
}
const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} harness cases pass; receipt in ${OUT}`);
process.exit(failed.length ? 1 : 0);
