//! The Rust model of the TradeX browser contract.
//!
//! This crate is the model, not the oracle: the G1 contract suite in `hub-core/tests/contract.rs`
//! compares raw JSON between the Python and Rust servers. Each module is owned by one Phase 01 plan
//! (`market` 01-01, `ws` and `health` 01-02, `logs` 01-03).

pub mod health;
pub mod logs;
pub mod market;
pub mod orderflow;
pub mod ws;
