// 03-03 browser check (AC-1..AC-5), driven against a test-idp hub-server on 127.0.0.1:8771.
// Run from the repo root: node .paul/runs/check-0303.mjs
// L-012: all state lives on the page object, never globalThis.
// PW_INDEX_MJS overrides the playwright module (CI: PW_INDEX_MJS=playwright with the
// package installed in the repo root; the default is the ux-ui kit copy on the dev box).
const { chromium } = await import(process.env.PW_INDEX_MJS ||
  "file:///C:/Users/Dell/.agents/ux-ui-agent-skills/node_modules/playwright/index.mjs");
import fs from "node:fs";
import path from "node:path";

const BASE = process.env.CHECK_BASE || "http://127.0.0.1:8771";
const OUT = path.resolve(".paul/runs/shots-0303");
fs.mkdirSync(OUT, { recursive: true });
const results = [];
const log = (ok, name, detail = "") => {
  results.push({ ok, name, detail });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? " :: " + detail : ""}`);
};

const GIS_STUB = `
window.google = { accounts: { id: {
  initialize(cfg) { window.__cfg = cfg; },
  prompt(cb) { window.__promptCb = cb; },
  cancel() {},
} } };
`;

async function newPage(browser, { scheme = "dark", width = 1280, height = 800 } = {}) {
  const ctx = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: 2 });
  const page = await ctx.newPage();
  await page.addInitScript((theme) => {
    // theme.js reads localStorage "chartHubTheme" before first paint.
    localStorage.setItem("chartHubTheme", JSON.stringify({ preset: theme }));
  }, scheme);
  // The GIS script never reaches Google: answer it with a stub that keeps the callback.
  await page.route("https://accounts.google.com/gsi/client", (route) =>
    route.fulfill({ status: 200, contentType: "text/javascript", body: GIS_STUB })
  );
  const seen = { logs: [], adminLogs: [], all: [] };
  page.on("request", (r) => {
    const u = new URL(r.url());
    if (!u.pathname.startsWith("/")) return;
    seen.all.push(`${r.method()} ${u.pathname}${u.search}`);
    if (u.pathname === "/api/logs") seen.logs.push(u.search);
    if (u.pathname === "/admin/logs") seen.adminLogs.push(u.search);
  });
  page.__seen = seen;
  return page;
}

async function signInViaMint(page, { sub = "g-web", email, nonce } = {}) {
  const body = { sub };
  if (email) body.email = email;
  if (nonce) body.nonce = nonce;
  const r = await page.request.post(`${BASE}/__test-idp/mint`, { data: body });
  const { credential } = await r.json();
  const res = await page.request.post(`${BASE}/auth/google/onetap`, {
    data: { credential },
    headers: { Origin: BASE, "Content-Type": "application/json" },
  });
  return res.status();
}

async function signOut(page) {
  return page.request.post(`${BASE}/auth/logout`, {
    data: {},
    headers: { Origin: BASE, "Content-Type": "application/json" },
  });
}

const shots = async (page, name) => page.screenshot({ path: path.join(OUT, `${name}.png`), fullPage: false });

// PW_CHANNEL overrides the launch channel (CI: chromium via `npx playwright install`;
// the dev box uses the system Chrome - the kit build's own chromium download is absent).
const browser = await chromium.launch({ channel: process.env.PW_CHANNEL || "chrome" });

// ---------- AC-3 local mode: charts page with auth off is a different server; assert hosted-mode
// counts here and read the local-mode behaviour from the separate local server if given.
// ---------- AC-1: signed out
{
  const page = await newPage(browser);
  await page.goto(`${BASE}/hub.html`, { waitUntil: "networkidle" });
  const btn = page.locator("#authSlot .auth-btn");
  log((await btn.count()) === 1, "AC-1 signed out: one sign-in button", await btn.innerText().catch(() => ""));
  log((await btn.getAttribute("aria-busy")) === null, "AC-1 signed out: not busy");
  const slotRight = await page.evaluate(() => {
    const s = document.getElementById("authSlot").getBoundingClientRect();
    const b = document.getElementById("bar").getBoundingClientRect();
    return { gap: b.right - s.right, w: s.width };
  });
  log(slotRight.gap >= 0 && slotRight.gap < 24, "AC-1 signed out: control at the bar's right end", JSON.stringify(slotRight));
  // 2.4.11 for the toolbar: no focusable control may sit under another at any width we claim.
  for (const w of [320, 768, 1280]) {
    await page.setViewportSize({ width: w, height: 800 });
    await page.waitForTimeout(150);
    const overlap = await page.evaluate(() => {
      // An element clipped by a scroll container is hidden, not occluded; only a point inside every
      // clipping ancestor's client box can be covered by something else.
      const clipOf = (el) => {
        let r = { left: 0, top: 0, right: innerWidth, bottom: innerHeight };
        for (let a = el.parentElement; a; a = a.parentElement) {
          const st = getComputedStyle(a);
          if (/(auto|hidden|scroll)/.test(st.overflowX) || /(auto|hidden|scroll)/.test(st.overflowY)) {
            const c = a.getBoundingClientRect();
            r = { left: Math.max(r.left, c.left), top: Math.max(r.top, c.top),
                  right: Math.min(r.right, c.right), bottom: Math.min(r.bottom, c.bottom) };
          }
        }
        return r;
      };
      const els = [...document.querySelectorAll("#bar button, #bar input, #bar select")].filter((e) => e.offsetParent !== null);
      const hits = [];
      for (const a of els) {
        const ra = a.getBoundingClientRect(), clip = clipOf(a);
        const left = Math.max(ra.left, clip.left), right = Math.min(ra.right, clip.right);
        const top_ = Math.max(ra.top, clip.top), bottom = Math.min(ra.bottom, clip.bottom);
        if (right - left < 2 || bottom - top_ < 2) continue;   // wholly scrolled out of view
        const cx = (left + right) / 2, cy = (top_ + bottom) / 2;
        const hit = document.elementFromPoint(cx, cy);
        if (hit && hit !== a && !a.contains(hit)) hits.push(`${a.id || a.textContent.trim().slice(0, 12)} under ${hit.id || hit.className}`);
      }
      return hits;
    });
    log(overlap.length === 0, `AC-4 ${w}px: no bar control covered by another`, JSON.stringify(overlap));
  }
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.waitForTimeout(150);
  log(page.__seen.logs.length === 0 && page.__seen.adminLogs.length === 0, "AC-3 signed out: zero log polls", JSON.stringify(page.__seen.logs.slice(0, 3)));
  log((await page.locator('#rail button[data-panel="logs"]').count()) === 0, "AC-3 signed out: no log rail entry");
  log((await page.locator("#logBadge").count()) === 0, "AC-3 signed out: no log badge");
  await shots(page, "01-signed-out-dark");
  // GIS stub got initialized with the public client id, and One Tap was prompted once.
  const gis = await page.evaluate(() => ({ cfg: window.__cfg ? window.__cfg.client_id : null, prompted: !!window.__promptCb }));
  log(gis.cfg === "test-client.apps.googleusercontent.com", "AC-2 GIS initialize carries /auth/me client_id", JSON.stringify(gis));
  log(gis.prompted, "AC-2 One Tap prompt was requested");

  // One Tap callback POSTs JSON {"credential"} to /auth/google/onetap
  const oneTap = await page.evaluate(async () => {
    const r = await fetch("/__test-idp/mint", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ sub: "g-onetap" }) });
    const { credential } = await r.json();
    let captured = null;
    const orig = window.fetch;
    window.fetch = (url, opts) => { if (String(url).includes("/auth/google/onetap")) captured = { url: String(url), opts }; return orig(url, opts); };
    window.__cfg.callback({ credential });
    await new Promise((r) => setTimeout(r, 600));
    window.fetch = orig;
    return { captured: captured && { url: captured.url, method: captured.opts.method, ctype: captured.opts.headers["Content-Type"], body: captured.opts.body } };
  });
  log(!!oneTap.captured, "AC-2 One Tap: POST issued");
  log(oneTap.captured && oneTap.captured.method === "POST" && String(oneTap.captured.ctype).includes("application/json"), "AC-2 One Tap POST is JSON", JSON.stringify(oneTap.captured && oneTap.captured.ctype));
  log(oneTap.captured && JSON.parse(oneTap.captured.body).credential?.split(".").length === 3, "AC-2 One Tap body is {credential}");
  await page.waitForTimeout(1200);
  const signedIn = (await page.locator("#authSlot > .avatar").count()) === 1;
  log(signedIn, "AC-2 One Tap signed the page in: avatar shown");
  if (signedIn) await shots(page, "02-onetap-signed-in-dark");
  await page.context().close();
}

// ---------- AC-1/AC-2 signed in: avatar menu, keyboard, sign out
{
  const page = await newPage(browser);
  await page.goto(`${BASE}/hub.html`, { waitUntil: "networkidle" });
  await signInViaMint(page, { sub: "g-menu", email: "sam.ortega@example.test" });
  await page.reload({ waitUntil: "networkidle" });
  const avatar = page.locator("#authSlot > .avatar");
  log((await avatar.count()) === 1, "AC-1 signed in: avatar trigger present");
  log((await avatar.getAttribute("aria-haspopup")) === "menu", "AC-1 avatar has aria-haspopup=menu");

  await avatar.click();
  const menu = page.locator("#authMenu");
  log(await menu.isVisible(), "AC-1 menu opens on click");
  log((await avatar.getAttribute("aria-expanded")) === "true", "AC-1 aria-expanded true while open");
  await shots(page, "03-menu-open-dark");
  const items = await page.locator("#authMenu [data-act]").evaluateAll((els) => els.map((e) => e.dataset.act));
  log(JSON.stringify(items) === JSON.stringify(["delete", "signout"]), "AC-1 member menu: delete + sign out, no Log panel", JSON.stringify(items));

  // Escape closes and returns focus to the trigger
  await page.keyboard.press("Escape");
  await page.waitForTimeout(150);
  log(!(await menu.isVisible()), "AC-1 Escape closes the menu");
  log(await page.evaluate(() => document.activeElement === document.querySelector("#authSlot > .avatar")), "AC-1 focus returned to the avatar");

  // ArrowDown on the trigger opens and focuses the first item
  await avatar.focus();
  await page.keyboard.press("ArrowDown");
  await page.waitForTimeout(150);
  log(await menu.isVisible(), "AC-1 ArrowDown opens the menu");
  log(await page.evaluate(() => document.activeElement?.dataset?.act === "delete"), "AC-1 ArrowDown focuses the first item");
  await page.keyboard.press("ArrowDown");
  log(await page.evaluate(() => document.activeElement?.dataset?.act === "signout"), "AC-1 ArrowDown moves to the next item");
  await page.keyboard.press("ArrowUp");
  log(await page.evaluate(() => document.activeElement?.dataset?.act === "delete"), "AC-1 ArrowUp moves back");
  await page.keyboard.press("End");
  log(await page.evaluate(() => document.activeElement?.dataset?.act === "signout"), "AC-1 End goes to the last item");
  await page.keyboard.press("Home");
  log(await page.evaluate(() => document.activeElement?.dataset?.act === "delete"), "AC-1 Home goes to the first item");
  await page.keyboard.press("Escape");

  // Sign out
  await avatar.click();
  const [logout] = await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/auth/logout")),
    page.locator('#authMenu [data-act="signout"]').click(),
  ]);
  log(logout.status() === 204, "AC-2 sign out is 204", String(logout.status()));
  await page.waitForTimeout(400);
  log((await page.locator("#authSlot .auth-btn").count()) === 1, "AC-2 after sign out the header shows the sign-in button");

  // SIGNOUT 401 shape: signed out state polling stops
  const before = page.__seen.logs.length + page.__seen.adminLogs.length;
  await page.waitForTimeout(5500);
  const afterSignOut = page.__seen.logs.length + page.__seen.adminLogs.length;
  log(afterSignOut === before, "AC-3 after sign out (member): still zero log polls");
  await page.context().close();
}

// ---------- AC-1/AC-3 admin: logs entry, /admin/logs polls, journal 200
{
  const page = await newPage(browser);
  await page.goto(`${BASE}/hub.html`, { waitUntil: "networkidle" });
  const st = await signInViaMint(page, { sub: "g-admin", email: "owner@example.test" });
  log(st === 200, "AC-2 admin mint signs in", String(st));
  await page.reload({ waitUntil: "networkidle" });
  const avatar = page.locator("#authSlot > .avatar");
  await avatar.click();
  const items = await page.locator("#authMenu [data-act]").evaluateAll((els) => els.map((e) => e.dataset.act));
  log(JSON.stringify(items) === JSON.stringify(["logs", "delete", "signout"]), "AC-1 admin menu has Log panel, Delete account, Sign out", JSON.stringify(items));
  log((await page.locator("#authMenu .badge").innerText()) === "Admin", "AC-1 admin badge in the menu header");
  await shots(page, "04-admin-menu-dark");

  await page.locator('#authMenu [data-act="logs"]').click();
  await page.waitForTimeout(600);
  log(await page.locator("#dock").isVisible(), "AC-3 admin: log entry opens the dock");
  log(page.__seen.adminLogs.length > 0, "AC-3 admin: polls /admin/logs", String(page.__seen.adminLogs.length));
  log(page.__seen.logs.length === 0, "AC-3 admin: zero /api/logs polls");

  const journal = await page.evaluate(async () => {
    const r = await fetch("/admin/logs/journal", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ raw: false }) });
    return r.status;
  });
  log(journal === 200, "AC-3 admin: journal POST is 200", String(journal));
  await shots(page, "05-admin-log-panel-dark");

  // Sign out, then back in as an admin: the gating must re-install exactly one entry and one badge.
  await page.request.post(`${BASE}/auth/logout`, { data: {}, headers: { Origin: BASE, "Content-Type": "application/json" } });
  await page.reload({ waitUntil: "networkidle" });
  await page.waitForTimeout(400);
  log((await page.locator('#rail button[data-panel="logs"]').count()) === 0, "AC-3 sign-out removes the rail entry");
  log((await page.locator("#logBadge").count()) === 0, "AC-3 sign-out removes the badge");
  await signInViaMint(page, { sub: "g-admin", email: "owner@example.test" });
  await page.reload({ waitUntil: "networkidle" });
  await page.waitForTimeout(600);
  log((await page.locator('#rail button[data-panel="logs"]').count()) === 1, "AC-3 re-sign-in installs exactly one rail entry");
  log((await page.locator("#logBadge").count()) === 1, "AC-3 re-sign-in installs exactly one badge");
  await page.context().close();
}

// ---------- AC-2 delete account: dialog, keyboard, Escape, 503 path
{
  const page = await newPage(browser);
  await page.goto(`${BASE}/hub.html`, { waitUntil: "networkidle" });
  await signInViaMint(page, { sub: "g-delete", email: "del@example.test" });
  await page.reload({ waitUntil: "networkidle" });
  await page.locator("#authSlot > .avatar").click();
  await page.locator('#authMenu [data-act="delete"]').click();
  const dlg = page.locator("dialog#authDel");
  log(await dlg.isVisible(), "AC-2 delete: dialog opens");
  log(await page.evaluate(() => document.activeElement?.getAttribute("value") === "cancel"), "AC-2 delete: focus starts on Cancel (safe action)");
  await shots(page, "06-delete-dialog-dark");

  // Focus is trapped inside the modal: Tab from the last control lands back inside.
  const trapped = await page.evaluate(() => {
    const d = document.querySelector("dialog#authDel");
    const inside = [];
    for (let i = 0; i < 6; i++) {
      const els = [...d.querySelectorAll("button")].filter((b) => !b.disabled);
      inside.push(document.activeElement && d.contains(document.activeElement));
      els[(i + 1) % els.length].focus();
    }
    return inside.every(Boolean);
  });
  log(trapped, "AC-2 delete: focus stays inside the modal dialog");
  const obscured = await page.evaluate(() => {
    const d = document.querySelector("dialog#authDel").getBoundingClientRect();
    const a = document.querySelector('[data-go]').getBoundingClientRect();
    return { top: d.top >= 0, bottom: d.bottom <= innerHeight, ctl: a.top >= 0 && a.bottom <= innerHeight };
  });
  log(obscured.top && obscured.bottom && obscured.ctl, "AC-4 2.4.11 focus not obscured in the dialog", JSON.stringify(obscured));

  // Escape cancels
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  log(!(await dlg.isVisible()), "AC-2 delete: Escape cancels");

  // 503 path: intercept DELETE /api/me
  await page.locator("#authSlot > .avatar").click();
  await page.locator('#authMenu [data-act="delete"]').click();
  await page.route("**/api/me", (route) =>
    route.request().method() === "DELETE"
      ? route.fulfill({ status: 503, contentType: "application/json", body: JSON.stringify({ error: "service unavailable" }) })
      : route.continue()
  );
  await dlg.locator("[data-go]").click();
  await page.waitForSelector("dialog#authDel .err:not([hidden])", { timeout: 5000 });
  const errText = await dlg.locator(".err").innerText();
  log(/not deleted and you are still signed in/.test(errText), "AC-2 delete: 503 shows the error state", errText.slice(0, 60));
  log((await dlg.locator("[data-go]").innerText()).toLowerCase().includes("try again"), "AC-2 delete: failure offers Try again");
  log((await dlg.locator('[data-go]').isDisabled()) === false, "AC-2 delete: buttons re-enabled after failure");
  await page.unroute("**/api/me");
  await shots(page, "07-delete-503-dark");

  // Happy path: DELETE 204 -> signed out
  await dlg.locator("[data-go]").click();
  await page.waitForFunction(() => !document.querySelector("dialog#authDel")?.open, null, { timeout: 8000 });
  await page.waitForTimeout(600);
  log(!(await dlg.isVisible()), "AC-2 delete: 204 closes the dialog");
  log((await page.locator("#authSlot .auth-btn").count()) === 1, "AC-2 delete: header returns to signed out");
  log((await page.locator("#authSlot > .avatar").count()) === 0, "AC-2 delete: avatar gone");
  const me = await page.evaluate(async () => (await (await fetch("/auth/me", { cache: "no-store" })).json()));
  log(me.user === null, "AC-2 delete: /auth/me reports signed out", JSON.stringify(me));
  await page.context().close();
}

// ---------- AC-1 sign-in failed notice
{
  const page = await newPage(browser);
  await page.goto(`${BASE}/hub.html?signin=failed`, { waitUntil: "networkidle" });
  await page.waitForTimeout(500);
  const n = page.locator("#authNotice");
  log(await n.isVisible(), "AC-1 sign-in failed: notice shown");
  log(/sign-in did not finish/i.test(await n.innerText()), "AC-1 sign-in failed: wording matches the mockup");
  log(page.url().endsWith("/hub.html"), "AC-1 sign-in failed: query stripped from the URL", page.url());
  await shots(page, "08-signin-failed-dark");
  await page.locator("#authNotice [data-dismiss]").click();
  log(!(await n.isVisible()), "AC-1 sign-in failed: Dismiss removes it");
  await page.context().close();
}

// ---------- AC-1/AC-4 light theme, 320px, and the primary button flow (button -> authorize -> callback)
{
  const page = await newPage(browser, { scheme: "light", width: 320, height: 720 });
  await page.goto(`${BASE}/hub.html`, { waitUntil: "networkidle" });
  const btn = page.locator("#authSlot .auth-btn");
  log(await btn.isVisible(), "AC-1 320px light: sign-in button visible");
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  log(overflow <= 0, "AC-4 320px light: no horizontal overflow", String(overflow));
  await shots(page, "09-signed-out-light-320");

  // Button flow: /auth/google/start -> test IdP authorize -> callback -> hub.html
  const chain = [];
  page.on("request", (r) => { const u = new URL(r.url()); if (u.pathname.startsWith("/auth/") || u.pathname.startsWith("/__test-idp/")) chain.push(u.pathname); });
  await Promise.all([page.waitForURL(/\/hub\.html/, { timeout: 15000 }), btn.click()]);
  await page.waitForTimeout(1200);
  const steps = chain.join(" > ");
  log(/\/auth\/google\/start/.test(steps) && /\/__test-idp\/authorize/.test(steps), "AC-2 button flow: start then authorize", steps);
  log((await page.locator("#authSlot > .avatar").count()) === 1, "AC-2 button flow: signed in, avatar shown", page.url());
  await page.locator("#authSlot > .avatar").click();
  await page.waitForTimeout(200);
  log(await page.locator("#authMenu").isVisible(), "AC-1 320px light: menu opens");
  const mOverflow = await page.evaluate(() => {
    const m = document.getElementById("authMenu").getBoundingClientRect();
    return { left: m.left, right: m.right, vw: innerWidth };
  });
  log(mOverflow.left >= 0 && mOverflow.right <= mOverflow.vw, "AC-4 320px light: menu stays in the viewport", JSON.stringify(mOverflow));
  await shots(page, "10-menu-light-320");
  await page.context().close();
}

// ---------- AC-2 One Tap dismissal flag survives within the tab
{
  const page = await newPage(browser);
  await page.goto(`${BASE}/hub.html`, { waitUntil: "networkidle" });
  const dismissed = await page.evaluate(() => {
    window.__promptCb({ isDismissedMoment: () => true, getDismissedReason: () => "credential_returned" });
    return sessionStorage.getItem("hubOnetapDismissed");
  });
  log(dismissed === "1", "AC-2 One Tap: a dismissal sets the per-tab flag", String(dismissed));
  await page.reload({ waitUntil: "networkidle" });
  const promptedAgain = await page.evaluate(() => !!window.__promptCb);
  log(!promptedAgain, "AC-2 One Tap: no prompt after dismissal in the same tab");
  await page.context().close();
}

await browser.close();

const failed = results.filter((r) => !r.ok);
fs.writeFileSync(path.join(OUT, "results.json"), JSON.stringify({ base: BASE, passed: results.length - failed.length, failed: failed.length, results }, null, 2));
console.log(`\n${results.length - failed.length}/${results.length} passed`);
if (failed.length) { console.log("FAILURES:"); failed.forEach((f) => console.log(` - ${f.name} :: ${f.detail}`)); process.exit(1); }
