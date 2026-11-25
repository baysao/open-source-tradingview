//! The one piece of state the shell carries.
//!
//! Small on purpose: a key, when it was last used, whether the page is ready to receive a delivered
//! code, and the queue of codes that arrived too early. The key never leaves Rust except through
//! `start_sign_in`, never reaches a log, and never touches the disk
//! (`crates/hub-core/src/auth/mod.rs:6` is the same discipline server-side).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long after opening a sign-in a second call reuses the key instead of opening another tab.
///
/// A double-click must not strand a one-time code in a browser tab the user then abandons. Five
/// seconds covers a double-click and an impatient second press, and never traps a real retry: the
/// user has to finish (or abandon) a browser round trip first, which takes longer.
pub const REOPEN_DEBOUNCE: Duration = Duration::from_secs(5);

pub struct DesktopState {
    key: Mutex<String>,
    last_start: Mutex<Option<Instant>>,
    /// Set on `PageLoadEvent::Finished`, cleared when a navigation starts. A delivery reads it to
    /// decide between dispatching now and queueing.
    pub page_ready: AtomicBool,
    /// Set when a code is delivered; `start_sign_in` clears it and mints a fresh key. The key a code
    /// is bound to must not outlive that code's use.
    pub needs_rotation: AtomicBool,
    /// Codes that arrived before the page was ready, in arrival order.
    pub pending: Mutex<VecDeque<String>>,
}

impl Default for DesktopState {
    fn default() -> Self {
        Self {
            key: Mutex::new(String::new()),
            last_start: Mutex::new(None),
            page_ready: AtomicBool::new(false),
            needs_rotation: AtomicBool::new(false),
            pending: Mutex::new(VecDeque::new()),
        }
    }
}

/// Whether a call should open a browser tab, given when the previous one was.
///
/// Pure so the debounce is testable without a clock: the caller passes `now`.
pub fn should_reopen(last_start: Option<Instant>, now: Instant) -> bool {
    match last_start {
        None => true,
        Some(prev) => now.duration_since(prev) >= REOPEN_DEBOUNCE,
    }
}

impl DesktopState {
    pub fn new() -> Self {
        Self::default()
    }

    /// The key to use for this call, and whether the caller should open a browser tab.
    ///
    /// The key is minted on the first call and kept afterwards; keeping it across a debounced
    /// repeat is what makes the repeat harmless. When a delivered code armed a rotation, the next
    /// call mints a fresh key instead of reusing the spent one.
    pub fn begin(&self, now: Instant) -> (String, bool) {
        let mut last = self.last_start.lock().expect("state lock");
        let reopen = should_reopen(*last, now);
        if reopen {
            *last = Some(now);
        }
        let mut key = self.key.lock().expect("state lock");
        let rotate = self.needs_rotation.swap(false, Ordering::AcqRel);
        if key.is_empty() || rotate {
            *key = mint_key();
        }
        (key.clone(), reopen)
    }
}

/// 32 OS random bytes, base64url without padding: 43 characters, inside the server's 22..=128.
fn mint_key() -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    let mut buf = [0u8; crate::config::KEY_BYTES];
    getrandom::getrandom(&mut buf).expect("OS random source");
    URL_SAFE_NO_PAD.encode(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_call_opens_and_later_ones_debounce() {
        let t0 = Instant::now();
        assert!(should_reopen(None, t0));
        assert!(!should_reopen(Some(t0), t0 + Duration::from_millis(10)));
        assert!(!should_reopen(Some(t0), t0 + Duration::from_secs(4)));
        assert!(should_reopen(Some(t0), t0 + REOPEN_DEBOUNCE));
        assert!(should_reopen(Some(t0), t0 + Duration::from_secs(30)));
    }

    #[test]
    fn the_key_is_minted_once_and_reused() {
        let st = DesktopState::new();
        let t0 = Instant::now();
        let (k1, reopen1) = st.begin(t0);
        assert!(reopen1);
        assert_eq!(k1.len(), 43);
        let (k2, reopen2) = st.begin(t0 + Duration::from_millis(100));
        assert!(!reopen2, "a double-click must not open a second tab");
        assert_eq!(
            k1, k2,
            "a debounced repeat reuses the key its code was minted for"
        );
        let (k3, reopen3) = st.begin(t0 + Duration::from_secs(10));
        assert!(reopen3, "a real retry opens again");
        assert_eq!(k1, k3);
    }

    #[test]
    fn a_delivered_code_forces_a_fresh_key_on_the_next_call() {
        let st = DesktopState::new();
        let t0 = Instant::now();
        let (k1, _) = st.begin(t0);
        // A code arrived, so the key it was bound to is spent.
        st.needs_rotation.store(true, Ordering::Release);
        let (k2, _) = st.begin(t0 + Duration::from_secs(10));
        assert_ne!(
            k1, k2,
            "the key that a used code was bound to must not be reused"
        );
        assert!(
            !st.needs_rotation.load(Ordering::Acquire),
            "the flag is consumed once"
        );
    }

    #[test]
    fn two_shells_mint_different_keys() {
        assert_ne!(
            DesktopState::new().begin(Instant::now()).0,
            DesktopState::new().begin(Instant::now()).0
        );
    }

    #[test]
    fn a_fresh_state_is_not_page_ready_and_has_nothing_queued() {
        let st = DesktopState::new();
        assert!(!st.page_ready.load(Ordering::Acquire));
        assert!(!st.needs_rotation.load(Ordering::Acquire));
        assert!(st.pending.lock().expect("lock").is_empty());
    }
}
