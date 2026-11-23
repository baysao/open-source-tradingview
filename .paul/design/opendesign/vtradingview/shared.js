// hub-native mockup runtime: icon sprite, preset switcher, --onAccent, toggles, menus, dialog focus trap.
(function () {
  // Lucide icon paths (ISC licence), drawn with currentColor.
  const I = {
    "log-in": '<path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4"/><path d="m10 17 5-5-5-5"/><path d="M15 12H3"/>',
    "log-out": '<path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/><path d="m16 17 5-5-5-5"/><path d="M21 12H9"/>',
    "cloud-check": '<path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z"/><path d="m9 14 2 2 4-4"/>',
    "cloud-off": '<path d="m2 2 20 20"/><path d="M5.78 5.78A7 7 0 0 0 9 19h8.5a4.5 4.5 0 0 0 1.31-.2"/><path d="M21.53 16.05A4.5 4.5 0 0 0 17.5 10h-1.79A7 7 0 0 0 10 5.07"/>',
    "refresh": '<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>',
    "check": '<path d="M20 6 9 17l-5-5"/>',
    "circle-check": '<circle cx="12" cy="12" r="10"/><path d="m9 12 2 2 4-4"/>',
    "alert": '<circle cx="12" cy="12" r="10"/><path d="M12 8v4"/><path d="M12 16h.01"/>',
    "triangle": '<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><path d="M12 9v4"/><path d="M12 17h.01"/>',
    "trash": '<path d="M3 6h18"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><path d="M10 11v6"/><path d="M14 11v6"/>',
    "plug": '<path d="M12 22v-5"/><path d="M9 8V2"/><path d="M15 8V2"/><path d="M18 8v5a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V8Z"/>',
    "activity": '<path d="M22 12h-4l-3 9L9 3l-3 9H2"/>',
    "rotate": '<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>',
    "x": '<path d="M18 6 6 18"/><path d="m6 6 12 12"/>',
    "eye": '<path d="M2.06 12.35a1 1 0 0 1 0-.7 10.75 10.75 0 0 1 19.88 0 1 1 0 0 1 0 .7 10.75 10.75 0 0 1-19.88 0"/><circle cx="12" cy="12" r="3"/>',
    "plus": '<path d="M5 12h14"/><path d="M12 5v14"/>',
    "external": '<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>',
    "shield": '<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/>',
    "undo": '<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11"/>',
    "pause": '<rect x="14" y="4" width="4" height="16" rx="1"/><rect x="6" y="4" width="4" height="16" rx="1"/>',
    "terminal": '<path d="m4 17 6-6-6-6"/><path d="M12 19h8"/>',
    "monitor": '<rect width="20" height="14" x="2" y="3" rx="2"/><path d="M8 21h8"/><path d="M12 17v4"/>',
    "search": '<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>',
    "settings": '<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>',
  };
  const sprite = '<svg xmlns="http://www.w3.org/2000/svg" style="display:none" aria-hidden="true">' +
    Object.entries(I).map(([k, v]) => `<symbol id="i-${k}" viewBox="0 0 24 24">${v}</symbol>`).join("") + "</svg>";
  document.body.insertAdjacentHTML("afterbegin", sprite);

  // --onAccent: whichever of accentInk and text contrasts more with accent. --lift: the colour opposite
  // to --onAccent, so hover and active fills move away from the label and never lose contrast.
  function rgb(c) {
    c = String(c).trim();
    if (c[0] === "#") { const n = parseInt(c.slice(1, 7), 16); return [(n >> 16) & 255, (n >> 8) & 255, n & 255]; }
    const m = c.match(/[\d.]+/g); return m ? m.slice(0, 3).map(Number) : [0, 0, 0];
  }
  function lum(c) { const [r, g, b] = rgb(c).map(v => { v /= 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); }); return 0.2126 * r + 0.7152 * g + 0.0722 * b; }
  function ratio(a, b) { const x = lum(a), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); }
  function derive() {
    const cs = getComputedStyle(document.documentElement), v = k => cs.getPropertyValue("--" + k).trim();
    const acc = v("accent"), ink = v("accentInk"), txt = v("text"), bg = v("bg");
    const on = ratio(ink, acc) >= ratio(txt, acc) ? "var(--accentInk)" : "var(--text)";
    const onVal = on === "var(--text)" ? txt : ink;
    const lift = ratio(txt, onVal) >= ratio(bg, onVal) ? "var(--text)" : "var(--bg)";
    const st = document.documentElement.style;
    st.setProperty("--onAccent", on); st.setProperty("--lift", lift);
  }
  window.MockApplyPreset = function (name) {
    const p = window.HubPresets && HubPresets[name]; if (!p) return;
    const st = document.documentElement.style;
    for (const k of HubThemeKeys) st.setProperty("--" + k, p[k]);
    st.colorScheme = p.mode; document.documentElement.dataset.preset = name;
    derive();
    document.querySelectorAll("[data-preset]").forEach(b => b.setAttribute("aria-pressed", String(b.dataset.preset === name)));
  };
  const host = document.querySelector(".presets");
  if (host && window.HubPresets) {
    host.innerHTML = '<span id="presetLbl">Theme preset</span>' + Object.keys(HubPresets).map(k =>
      `<button type="button" class="btn" data-preset="${k}" aria-pressed="false">${k[0].toUpperCase() + k.slice(1)}</button>`).join("");
    host.setAttribute("role", "group"); host.setAttribute("aria-labelledby", "presetLbl");
    host.addEventListener("click", e => { const b = e.target.closest("[data-preset]"); if (b) MockApplyPreset(b.dataset.preset); });
    const q = new URLSearchParams(location.search).get("preset");
    MockApplyPreset(q && HubPresets[q] ? q : (window.HubTheme && HubTheme.preset) || "graphite");
  } else derive();

  // Disclosure: [aria-controls][aria-expanded] toggles the target's hidden attribute.
  document.addEventListener("click", e => {
    const t = e.target.closest("[aria-expanded][aria-controls]");
    if (t) { const el = document.getElementById(t.getAttribute("aria-controls")); const open = t.getAttribute("aria-expanded") !== "true";
      t.setAttribute("aria-expanded", String(open)); if (el) el.hidden = !open;
      if (open && el && el.getAttribute("role") === "menu") { const f = el.querySelector('[role="menuitem"]'); if (f && e.detail === 0) f.focus(); } }
    const p = e.target.closest(".tog[aria-pressed]");
    if (p) { const on = p.getAttribute("aria-pressed") !== "true"; p.setAttribute("aria-pressed", String(on)); if (p.dataset.on) p.querySelector("b").textContent = on ? p.dataset.on : p.dataset.off; }
    const cl = e.target.closest("[data-collapse]");
    if (cl) { const id = cl.dataset.collapse, el = document.getElementById(id); if (el) el.hidden = true;
      document.querySelectorAll(`[aria-controls="${id}"]`).forEach(t => { t.setAttribute("aria-expanded", "false"); t.focus(); }); }
    const r = e.target.closest("[data-reveal]");
    if (r) { const el = document.getElementById(r.dataset.reveal); if (el) el.hidden = !el.hidden; }
    const pw = e.target.closest("[data-showpw]");
    if (pw) { const inp = document.getElementById(pw.dataset.showpw); const show = inp.type === "password"; inp.type = show ? "text" : "password";
      pw.setAttribute("aria-pressed", String(show)); pw.setAttribute("aria-label", show ? "Hide token" : "Show token"); }
  });

  // Menu keyboard model: arrows move, Home/End jump, Escape closes and returns focus to the trigger.
  document.addEventListener("keydown", e => {
    const menu = e.target.closest('[role="menu"]'); if (!menu) return;
    const items = [...menu.querySelectorAll('[role="menuitem"]:not(:disabled)')]; const i = items.indexOf(e.target);
    const go = n => { e.preventDefault(); items[(n + items.length) % items.length].focus(); };
    if (e.key === "ArrowDown") go(i + 1); else if (e.key === "ArrowUp") go(i - 1);
    else if (e.key === "Home") go(0); else if (e.key === "End") go(items.length - 1);
    else if (e.key === "Escape") { const t = document.querySelector(`[aria-controls="${menu.id}"]`); if (t) { t.setAttribute("aria-expanded", "false"); menu.hidden = true; t.focus(); } }
  });
  document.querySelectorAll('[role="menu"]').forEach(m => m.querySelectorAll('[role="menuitem"]').forEach((it, k) => it.tabIndex = k === 0 ? 0 : -1));

  // Live modal: [data-open-dialog] opens a .scrim.live; focus is trapped, Escape closes, focus returns.
  let opener = null;
  function closeDlg(s) { s.hidden = true; if (opener) opener.focus(); }
  document.addEventListener("click", e => {
    const o = e.target.closest("[data-open-dialog]");
    if (o) { const s = document.getElementById(o.dataset.openDialog); opener = o; s.hidden = false; const f = s.querySelector("[data-autofocus]") || s.querySelector("button"); f && f.focus(); }
    const c = e.target.closest("[data-close-dialog]");
    if (c) closeDlg(c.closest(".scrim"));
  });
  document.addEventListener("keydown", e => {
    const s = document.querySelector(".scrim.live:not([hidden])"); if (!s) return;
    if (e.key === "Escape") { e.preventDefault(); closeDlg(s); return; }
    if (e.key !== "Tab") return;
    const f = [...s.querySelectorAll("button:not(:disabled), [href], input:not(:disabled), [tabindex]:not([tabindex='-1'])")];
    if (!f.length) return; const a = f[0], z = f[f.length - 1];
    if (e.shiftKey && document.activeElement === a) { e.preventDefault(); z.focus(); }
    else if (!e.shiftKey && document.activeElement === z) { e.preventDefault(); a.focus(); }
  });
})();
