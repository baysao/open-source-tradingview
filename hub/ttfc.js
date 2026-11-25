// TTFC: time to first live candle (navigation start -> first live bar drawn on an engine page).
//
// Each engine page loads this with data-engine="<name>" and calls HubTTFC.send() once, at its
// first live bar. hub.html loads it with no data-engine: it frames the engines and measures
// nothing, so send() there is a no-op and the page sends no beacon at all.
//
// The clock is performance.now(), milliseconds since this page's navigation start, so no
// timestamp has to be captured at the top of the page's own block. One POST per load: the server
// records it in a fixed histogram (crates/hub-core/src/routes/metrics.rs).
(() => {
  if (window.HubTTFC) return; // a page that loaded it twice: the first copy owns the beacon
  const ENGINES = ["vela", "lightweight", "klinechart"];
  const MAX_MS = 120000; // the server's top bucket: a slower load is not a TTFC sample
  const me = document.currentScript;
  const engine = me && ENGINES.includes(me.dataset.engine) ? me.dataset.engine : "";
  let sent = false;
  window.HubTTFC = {
    engine,
    send() {
      if (sent || !engine) return;
      const ms = Math.round(performance.now());
      // A restored background tab reports minutes: drop it rather than let it poison the p95.
      if (!(ms >= 0 && ms <= MAX_MS)) return;
      sent = true;
      try {
        // fetch keepalive, not sendBeacon: this is the same POST but with the JSON content type
        // the server's post_guard requires (sendBeacon would send text/plain and get a 415).
        fetch("/api/metrics/ttfc", {
          method: "POST",
          keepalive: true,
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ ms, engine }),
        }).catch(() => {});
      } catch (_) { /* a blocked beacon never breaks the chart */ }
    },
  };
})();
