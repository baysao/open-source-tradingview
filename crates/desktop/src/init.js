// The remote page's entire desktop surface. One function, non-writable and non-configurable:
// a page that could replace it could impersonate the shell, and a page that could add to it
// would have a second channel nobody audited.
//
// `window.__TAURI_INTERNALS__` is present even with `withGlobalTauri: false` — that flag controls
// only the friendly `window.__TAURI__` namespace. The ACL is what actually gates every call, so
// this script being injectable is not the boundary; the capability is.
Object.defineProperty(window, "vtvDesktop", {
  value: Object.freeze({
    startSignIn: () => window.__TAURI_INTERNALS__.invoke("start_sign_in"),
  }),
  writable: false,
  configurable: false,
  enumerable: false,
});
