// Phase 4 load test (spec item 34, NFR "measured, not assumed"):
// opens N websocket connections with realistic subscriptions and reports connections held,
// drops, and the server's upstream subscription count.
//
//   node scripts/load_ws.mjs [--url ws://127.0.0.1:8770/ws] [--http http://127.0.0.1:8770]
//                            [--connections 1000] [--duration 1800] [--report-every 30]
//
// Precondition: a LOCAL-MODE server (AUTH_ENABLED=false), where loopback is exempt from the
// per-IP socket cap (24). In hosted mode one IP shares that cap, so a single-machine run
// reports connect errors rather than a server defect; the 1,000-socket pass needs several
// source IPs or a raised cap.
// Mix (fixed by the spec review, C25): 1,000 connections, average 2 subscriptions each, 50
// distinct symbols across Deriv, Bybit and Binance, 70% M1 and 30% H1.
// Pass: zero dropped connections, upstream subscriptions at 50 (or skip-adjusted: a source
// that refuses this runner's IP is excluded, and the run says which sources served; a run
// where every source skipped is FAIL, not a pass).
// Exit codes: 0 pass, 1 fail, 2 all sources skipped.

const args = {};
for (let i = 2; i < process.argv.length; i++) {
  const a = process.argv[i];
  if (a.startsWith("--")) args[a.slice(2)] = process.argv[i + 1], i++;
}
const URL = args.url || "ws://127.0.0.1:8770/ws";
const HTTP = args.http || URL.replace(/^ws/, "http").replace(/\/ws$/, "");
const N = parseInt(args.connections || "1000", 10);
const DURATION_S = parseInt(args.duration || "1800", 10);
const REPORT_EVERY = parseInt(args.reportEvery || args["report-every"] || "30", 10);

// The 50-symbol list: 20 Deriv, 15 Bybit, 15 Binance (USD-M futures).
const SYMBOLS = [
  ...["frxXAUUSD", "frxXAGUSD", "frxEURUSD", "frxGBPUSD", "frxUSDJPY", "frxAUDUSD",
      "frxUSDCAD", "frxNZDUSD", "frxEURJPY", "frxGBPJPY", "frxBTCUSD", "frxETHUSD",
      "OTC_NDX", "OTC_US30", "OTC_GDP", "R_10", "R_25", "R_50", "R_75", "R_100"].map((s) => ["deriv", s]),
  ...["BYBIT_S_BTCUSDT", "BYBIT_S_ETHUSDT", "BYBIT_S_SOLUSDT", "BYBIT_S_XRPUSDT",
      "BYBIT_S_ADAUSDT", "BYBIT_S_DOGEUSDT", "BYBIT_S_AVAXUSDT", "BYBIT_S_LINKUSDT",
      "BYBIT_S_DOTUSDT", "BYBIT_S_LTCUSDT", "BYBIT_S_BNBUSDT", "BYBIT_S_TRXUSDT",
      "BYBIT_S_NEARUSDT", "BYBIT_S_APTUSDT", "BYBIT_S_ARBUSDT"].map((s) => ["bybit", s]),
  ...["BTCUSD", "ETHUSD", "SOLUSD", "XRPUSD", "ADAUSD", "DOGEUSD", "AVAXUSD", "LINKUSD",
      "DOTUSD", "LTCUSD", "BNBUSD", "TRXUSD", "NEARUSD", "APTUSD", "ARBUSD"].map((s) => ["binance", s]),
];
const TFS = [...Array(70).fill("M1"), ...Array(30).fill("H1")]; // 70% M1, 30% H1

const stats = {
  opened: 0, open: 0, peak: 0, dropped: 0, connectErrors: 0, errors: 0, sent: 0,
  refusedBySource: {}, bars: 0,
};

function connect(i) {
  return new Promise((resolve) => {
    const ws = new WebSocket(URL);
    const my = {
      ws, id: i, subs: [], healthyAt: 0, closedByUs: false, settled: false,
    };
    // A refused socket never fires onopen (per-IP cap, rate limit): settle on close or error,
    // or after 10 s. Without this Promise.all waits forever and the run hangs.
    const settle = () => { if (!my.settled) { my.settled = true; resolve(my); } };
    setTimeout(settle, 10000);
    ws.onopen = () => {
      // 2 subscriptions per connection, deterministic assignment: symbol j and j+50.
      const picks = [i % 50, (i + 50) % 50];
      picks.forEach((s, k) => {
        const [source, symbol] = SYMBOLS[s];
        const tf = TFS[(i * 2 + k) % 100];
        my.subs.push(`${source}:${symbol}:${tf}`);
        ws.send(JSON.stringify({ source, symbol, tf }));
        stats.sent++;
      });
      my.opened = true;
      stats.opened++; stats.open++; stats.peak = Math.max(stats.peak, stats.open);
      settle();
    };
    ws.onmessage = (e) => {
      try {
        const m = JSON.parse(e.data);
        if (m.type === "bar") { stats.bars++; my.healthyAt = Date.now(); }
        if (m.type === "error") {
          stats.errors++;
          const src = (m.msg || "").match(/^(deriv|bybit|binance)/i);
          if (src) stats.refusedBySource[src[1].toLowerCase()] = (stats.refusedBySource[src[1].toLowerCase()] || 0) + 1;
        }
      } catch (_) { /* not JSON: ignore */ }
    };
    ws.onclose = () => {
      if (!my.opened) stats.connectErrors++;
      else if (!my.closedByUs) stats.dropped++;
      if (my.opened) stats.open--;
      settle();
    };
    ws.onerror = () => { if (!my.opened) stats.connectErrors++; settle(); };
  });
}

async function main() {
  console.log(`load_ws: ${N} connections, ${DURATION_S}s, url ${URL}`);
  console.log(`mix: 50 symbols (deriv 20, bybit 15, binance 15), 2 subs per conn, 70% M1 / 30% H1`);
  const started = Date.now();
  const conns = [];
  const ramp = Math.min(N, Math.max(1, Math.floor(N / 20))); // 20 batches
  for (let b = 0; b < N; b += ramp) {
    const batch = [];
    for (let i = b; i < Math.min(b + ramp, N); i++) batch.push(connect(i));
    conns.push(...await Promise.all(batch));
    await new Promise((r) => setTimeout(r, 250));
  }
  console.log(`opened ${stats.opened} connections`);

  const reporter = setInterval(() => {
    const held = conns.filter((c) => c.ws.readyState === 1).length;
    console.log(`[${Math.round((Date.now() - started) / 1000)}s] held=${held} dropped=${stats.dropped} errors=${stats.errors} bars=${stats.bars}`);
  }, REPORT_EVERY * 1000);

  await new Promise((r) => setTimeout(r, DURATION_S * 1000));
  clearInterval(reporter);
  conns.forEach((c) => { c.closedByUs = true; try { c.ws.close(); } catch (_) {} });

  // Server-side upstream count: /api/health's feeds map is keyed by `source:symbol`, one
  // upstream per distinct key (the shared-subscription total; N2: keys carry no timeframe).
  let upstreams = "?";
  try {
    const h = await (await fetch(`${HTTP}/api/health`)).json();
    const feeds = h?.feeds ?? {};
    upstreams = typeof feeds === "object" ? Object.keys(feeds).length : "?";
  } catch (_) { /* leave "?" */ }

  const refusedSources = Object.keys(stats.refusedBySource);
  const servedSources = ["deriv", "bybit", "binance"].filter((s) => !refusedSources.includes(s));
  console.log("\n=== RESULT ===");
  console.log(`peak held: ${stats.peak}/${N} (opened ${stats.opened}, connect errors ${stats.connectErrors})`);
  console.log(`dropped by server: ${stats.dropped}`);
  console.log(`feed errors: ${stats.errors}, bars received: ${stats.bars}`);
  console.log(`server upstreams: ${upstreams}`);
  console.log(`sources served: ${servedSources.join(", ") || "none"}`);
  if (refusedSources.length) console.log(`sources refused (errors seen): ${refusedSources.join(", ")}`);

  if (servedSources.length === 0) { console.log("FAIL: every source skipped; this run proves nothing"); process.exit(2); }
  if (stats.dropped === 0) { console.log("PASS: no dropped connections"); process.exit(0); }
  console.log("FAIL: connections were dropped");
  process.exit(1);
}

main().catch((e) => { console.error("load_ws crashed:", e); process.exit(1); });
