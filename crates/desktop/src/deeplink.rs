//! The `vtradingview://auth?code=<code>` hand-off.
//!
//! The browser finishes Google's sign-in and redirects to the deep link
//! (`crates/hub-core/src/routes/auth.rs:82` formats exactly that URL). On Windows and Linux a
//! protocol URL launches a *new process* whose only argument is the URL, so delivery happens on
//! three paths: the running instance receives it through `single-instance`, a cold start reads it
//! from `deep_link().get_current()`, and a link that arrives before the page has loaded waits in
//! [`DesktopState::pending`].
//!
//! Delivery is a Rust-side `eval`, not a navigation and not an event permission (D-029): the page's
//! `desktop_key` lives in page memory, and a reload would destroy it while the one-time code still
//! had life left.

use std::sync::atomic::Ordering;

use serde_json::json;

use crate::state::DesktopState;

/// The minimum and maximum code length the parser tolerates.
///
/// The floor is 16, deliberately **below** the server's 22 (`crates/hub-core/src/auth/rules.rs`
/// `parse_client` accepts 22..=128 for a `desktop_key`; a minted code is 43). Widening here is not a
/// relaxation of the real guard — the server refuses any code it did not mint, and a code arrives
/// only after a successful OAuth round trip. Tightening the client to 22 would instead convert a
/// future server-side length change into a silently dead shell, which is the failure this parser is
/// meant to make visible.
const CODE_MIN: usize = 16;
const CODE_MAX: usize = 128;

/// Parses `vtradingview://auth?code=<code>`.
///
/// Returns `None` for anything that is not exactly that shape. Never a partial match: a wrong
/// scheme, a wrong host, a missing or duplicated `code`, or a value outside `[A-Za-z0-9_-]` is a
/// refusal, and the caller logs it.
pub fn parse_code(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;

    if parsed.scheme() != crate::config::DEEP_LINK_SCHEME {
        return None;
    }
    // `vtradingview://auth?...` puts `auth` in the host position. A path-based link
    // (`vtradingview:///auth?code=`) has an empty host and is refused: the server writes the host
    // form, and accepting both would mean two shapes to audit.
    if parsed.host_str() != Some("auth") {
        return None;
    }

    // Count rather than take the first: a second `code` is parameter smuggling, and `Url::query_pairs`
    // silently yields both.
    let codes: Vec<String> = parsed
        .query_pairs()
        .filter(|(k, _)| k == "code")
        .map(|(_, v)| v.into_owned())
        .collect();
    if codes.len() != 1 {
        return None;
    }

    let code = codes[0].as_str();
    // `Url::query_pairs` percent-decodes, so `%41` is `A` by the time it reaches here. That is
    // correct and harmless — the check runs on the *decoded* value, which is the same string the
    // server hashed, so an encoded spelling of a valid code is still that code. What must not slip
    // through is an encoding that decodes to something illegal, so the charset check below is the
    // one that matters: `%2F` decodes to `/` and is refused here.
    if code.len() < CODE_MIN || code.len() > CODE_MAX {
        return None;
    }
    if !code
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }

    Some(code.to_string())
}

/// What delivers the event into the page. A trait so the queue can be tested without a window.
pub trait Emitter {
    fn eval(&self, js: &str) -> Result<(), String>;
}

/// The production emitter: the page's own window.
pub struct WindowEmitter<'a>(pub &'a tauri::WebviewWindow);

impl Emitter for WindowEmitter<'_> {
    fn eval(&self, js: &str) -> Result<(), String> {
        self.0.eval(js).map_err(|e| e.to_string())
    }
}

/// The event name, fixed here so 06-03 codes against one spelling.
pub const EVENT: &str = "vtv:desktop-code";

/// Builds the JS payload. The code is the only interpolated value and `parse_code` has already
/// restricted it to `[A-Za-z0-9_-]`; `serde_json` doubles the escaping anyway, because an
/// interpolation bug here would be script injection into the app window.
pub fn event_js(code: &str) -> String {
    let detail = json!({ "code": code }).to_string();
    format!("window.dispatchEvent(new CustomEvent('{EVENT}', {{detail: {detail}}}))")
}

/// Delivers a code to the page, or queues it until the page reports a finished load.
///
/// Returns whether it was delivered now. Errors from `eval` leave the code queued rather than
/// dropping it: the code has 60 s of life, so a retry is harmless, while losing it makes the user
/// repeat the whole browser round trip.
pub fn deliver(state: &DesktopState, emitter: &dyn Emitter, code: &str) -> bool {
    // The next `start_sign_in` should mint a fresh key: this code is bound to the current one.
    state.needs_rotation.store(true, Ordering::Release);

    if !state.page_ready.load(Ordering::Acquire) {
        state
            .pending
            .lock()
            .expect("state lock")
            .push_back(code.to_string());
        tracing::debug!(target: "desktop", "deep link queued ({} chars, page not ready)", code.len());
        return false;
    }

    match emitter.eval(&event_js(code)) {
        Ok(()) => {
            // Length only: never the code itself.
            tracing::info!(target: "desktop", "deep link delivered ({} chars)", code.len());
            true
        }
        Err(e) => {
            tracing::warn!(target: "desktop", "deep link delivery failed, queued: {e}");
            state
                .pending
                .lock()
                .expect("state lock")
                .push_back(code.to_string());
            false
        }
    }
}

/// Marks the page ready and drains the queue in arrival order.
///
/// `page_ready` is set **before** draining, so a link arriving during the drain queues rather than
/// racing the loop. AC-3's two-links case is the test for that ordering.
pub fn flush(state: &DesktopState, emitter: &dyn Emitter) {
    state.page_ready.store(true, Ordering::Release);
    loop {
        let next = state.pending.lock().expect("state lock").pop_front();
        let Some(code) = next else { break };
        if emitter.eval(&event_js(&code)).is_err() {
            // Put it back at the front and stop; a later load will retry.
            state.pending.lock().expect("state lock").push_front(code);
            break;
        }
        tracing::info!(target: "desktop", "queued deep link delivered ({} chars)", code.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const GOOD: &str = "abcdefghij_klmnopqrstu"; // 22 chars, the server's floor

    #[test]
    fn the_servers_shape_is_accepted() {
        assert_eq!(
            parse_code(&format!("vtradingview://auth?code={GOOD}")).as_deref(),
            Some(GOOD)
        );
    }

    #[test]
    fn short_codes_and_wrong_shapes_are_refused() {
        assert_eq!(parse_code("vtradingview://auth?code=short"), None);
        assert_eq!(
            parse_code(&format!("https://vtradingview.com/auth?code={GOOD}")),
            None,
            "a web URL is not a deep link"
        );
        assert_eq!(
            parse_code(&format!("vtradingview://other?code={GOOD}")),
            None
        );
        assert_eq!(
            parse_code(&format!("vtradingview://auth?code={GOOD}&code={GOOD}")),
            None,
            "a second code is parameter smuggling"
        );
        assert_eq!(parse_code("vtradingview://auth"), None);
        assert_eq!(parse_code("vtradingview://auth?code="), None);
    }

    #[test]
    fn the_bounds_are_enforced() {
        assert_eq!(
            parse_code(&format!("vtradingview://auth?code={}", "a".repeat(15))),
            None
        );
        assert!(parse_code(&format!("vtradingview://auth?code={}", "a".repeat(16))).is_some());
        assert!(parse_code(&format!("vtradingview://auth?code={}", "a".repeat(128))).is_some());
        assert_eq!(
            parse_code(&format!("vtradingview://auth?code={}", "a".repeat(129))),
            None
        );
    }

    #[test]
    fn a_percent_encoded_or_illegal_code_is_refused() {
        // Measured: `query_pairs` decodes, so `%41..` is the valid code `ABCDEFGHIJKLMNOP` and is
        // accepted — the same value the server hashed, so nothing is smuggled. The refusals that
        // matter are encodings that decode to an illegal character.
        assert_eq!(
            parse_code("vtradingview://auth?code=%41%42%43%44%45%46%47%48%49%4a%4b%4c%4d%4e%4f%50"),
            Some("ABCDEFGHIJKLMNOP".to_string())
        );
        // `%2F` is `/`, outside the base64url alphabet.
        assert_eq!(
            parse_code(&format!("vtradingview://auth?code={GOOD}%2Fx")),
            None
        );
        // A slash and a space, written literally.
        assert_eq!(
            parse_code(&format!("vtradingview://auth?code={GOOD}/x")),
            None
        );
        assert_eq!(
            parse_code(&format!("vtradingview://auth?code={GOOD}%20x")),
            None
        );
    }

    /// Records what it was asked to evaluate, and can be told to fail.
    struct Spy {
        seen: RefCell<Vec<String>>,
        fail: bool,
    }
    impl Spy {
        fn new() -> Self {
            Self {
                seen: RefCell::new(Vec::new()),
                fail: false,
            }
        }
        fn failing() -> Self {
            Self {
                seen: RefCell::new(Vec::new()),
                fail: true,
            }
        }
        fn codes(&self) -> Vec<String> {
            self.seen
                .borrow()
                .iter()
                .map(|js| {
                    let at = js.find("\"code\":\"").expect("payload has a code") + 8;
                    js[at..]
                        .split('"')
                        .next()
                        .expect("code is terminated")
                        .to_string()
                })
                .collect()
        }
    }
    impl Emitter for Spy {
        fn eval(&self, js: &str) -> Result<(), String> {
            if self.fail {
                return Err("boom".into());
            }
            self.seen.borrow_mut().push(js.to_string());
            Ok(())
        }
    }

    #[test]
    fn a_link_before_the_page_is_ready_is_queued_then_delivered_on_flush() {
        let st = DesktopState::new();
        let spy = Spy::new();
        assert!(!deliver(&st, &spy, "aaaaaaaaaaaaaaaaaaaa"));
        assert!(
            spy.codes().is_empty(),
            "nothing delivered before the page is ready"
        );
        flush(&st, &spy);
        assert_eq!(spy.codes(), vec!["aaaaaaaaaaaaaaaaaaaa"]);
    }

    #[test]
    fn two_queued_links_arrive_in_order_and_only_once() {
        let st = DesktopState::new();
        let spy = Spy::new();
        deliver(&st, &spy, "aaaaaaaaaaaaaaaaaaaa");
        deliver(&st, &spy, "bbbbbbbbbbbbbbbbbbbb");
        flush(&st, &spy);
        assert_eq!(
            spy.codes(),
            vec!["aaaaaaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbbbbbb"]
        );
        // A second load must not replay them.
        flush(&st, &spy);
        assert_eq!(spy.codes().len(), 2, "the queue drained exactly once");
    }

    #[test]
    fn a_ready_page_gets_the_event_immediately() {
        let st = DesktopState::new();
        st.page_ready.store(true, Ordering::Release);
        let spy = Spy::new();
        assert!(deliver(&st, &spy, "cccccccccccccccccccc"));
        assert_eq!(spy.codes(), vec!["cccccccccccccccccccc"]);
    }

    #[test]
    fn a_failed_delivery_is_kept_rather_than_dropped() {
        let st = DesktopState::new();
        st.page_ready.store(true, Ordering::Release);
        let spy = Spy::failing();
        assert!(!deliver(&st, &spy, "dddddddddddddddddddd"));
        assert_eq!(
            st.pending.lock().expect("lock").len(),
            1,
            "the code is still queued"
        );
    }

    #[test]
    fn delivery_arms_a_key_rotation() {
        let st = DesktopState::new();
        st.page_ready.store(true, Ordering::Release);
        let spy = Spy::new();
        assert!(!st.needs_rotation.load(Ordering::Acquire));
        deliver(&st, &spy, "eeeeeeeeeeeeeeeeeeee");
        assert!(
            st.needs_rotation.load(Ordering::Acquire),
            "the next sign-in mints a fresh key"
        );
    }

    #[test]
    fn a_link_arriving_during_the_drain_is_delivered_not_lost() {
        // The property mutation m4 breaks: `page_ready` must be set BEFORE the drain, so a delivery
        // that happens while the queue is being emptied takes the immediate path.
        //
        // Two assertions, because the interleaved one alone was not enough to catch m4 (measured —
        // the first attempt passed under the mutation):
        //   (a) the mid-drain arrival is delivered, and
        //   (b) `page_ready` is already true for the FIRST queued delivery, which is the observable
        //       difference between the two orderings and cannot be faked by luck of scheduling.
        struct Probe {
            state: std::sync::Arc<DesktopState>,
            ready_at_first_eval: RefCell<Option<bool>>,
            seen: RefCell<Vec<String>>,
            inject: bool,
        }
        impl Emitter for Probe {
            fn eval(&self, js: &str) -> Result<(), String> {
                let mut first = self.ready_at_first_eval.borrow_mut();
                if first.is_none() {
                    *first = Some(self.state.page_ready.load(Ordering::Acquire));
                }
                drop(first);
                let at = js.find("\"code\":\"").expect("payload has a code") + 8;
                let code = js[at..].split('"').next().expect("terminated").to_string();
                self.seen.borrow_mut().push(code);
                if self.inject && self.seen.borrow().len() == 1 {
                    // Mid-drain arrival, made deterministic.
                    deliver(&self.state, self, "zzzzzzzzzzzzzzzzzzzz");
                }
                Ok(())
            }
        }

        let st = std::sync::Arc::new(DesktopState::new());
        deliver(&st, &Spy::new(), "aaaaaaaaaaaaaaaaaaaa");
        let probe = Probe {
            state: std::sync::Arc::clone(&st),
            ready_at_first_eval: RefCell::new(None),
            seen: RefCell::new(Vec::new()),
            inject: true,
        };
        flush(&st, &probe);

        assert_eq!(
            *probe.ready_at_first_eval.borrow(),
            Some(true),
            "(b) page_ready must already be true when the drain begins"
        );
        let codes = probe.seen.borrow().clone();
        assert!(
            codes.contains(&"zzzzzzzzzzzzzzzzzzzz".to_string()),
            "(a) a link arriving mid-drain must be delivered, got {codes:?}"
        );
    }

    #[test]
    fn the_payload_escapes_the_code() {
        // Measured: serde_json escapes `"` and `\` but not `'`, which is correct — the payload sits
        // inside a DOUBLE-quoted JSON string, so a single quote cannot break out. A double quote is
        // the one that could, and it must arrive escaped.
        let js = event_js(r#"a"-alert(1)-"b"#);
        assert!(
            js.contains(r#"\"-alert(1)-"#),
            "the double quote must be escaped: {js}"
        );
        // The surrounding JS string literal is single-quoted, so a single quote would break THAT.
        // parse_code never allows one through, but the builder must not rely on it.
        let js = event_js("a'-alert(1)-'b");
        assert!(
            !js.contains("'-alert(1)-'b'"),
            "a single quote must not close the literal: {js}"
        );
    }
}
