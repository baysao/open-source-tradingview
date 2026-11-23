//! Live feeds: one upstream per `(source, symbol)`, a bar builder per timeframe, broadcast fan-out
//! (D-002). `state` is pure and synchronous; `hub` and `upstream` are the thin async shell.

pub mod hub;
pub mod state;
pub mod upstream;

pub use hub::{FeedHub, SocketPermit, Subscription, Wanted};
pub use state::{FeedView, Limits, Refusal};
