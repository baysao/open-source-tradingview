//! Authentication: pure decisions ([`rules`]), sealed secrets ([`ids`]) and, under the
//! `test-idp` feature, the in-process identity provider ([`test_idp`]).
//!
//! Layering: `rules` and `ids` are the core (no I/O, table-testable); `test_idp` mints real
//! RS256 tokens so the suites can exercise the token edges without ever calling Google.
//
// The store, Google client and gate land in the next tasks of 03-02; everything here is the
// table-tested core those tasks wire up, so `dead_code` holds until then.
#![allow(dead_code)]

pub mod ids;
pub mod rules;

#[cfg(feature = "test-idp")]
pub mod test_idp;
