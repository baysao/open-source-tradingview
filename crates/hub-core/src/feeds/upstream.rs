//! One session loop for every provider, driven by its `LivePlan`: connect, send the hello frames,
//! keep alive, read, parse outside the lock, ingest under it, fan out; on end broadcast a `status`
//! notice, back off, reconnect. History is not refetched after a reconnect (Python parity).

use super::hub::{frame, FeedHub};
use super::state::UpstreamStart;
use crate::market::backoff::Backoff;
use crate::sources::{self, KeepAlive, LivePlan, Parsed};
use futures_util::{SinkExt, StreamExt};
use hub_types::ws::ServerMsg;
use std::time::Duration;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;

enum End {
    /// The provider ended the session with a notice (Deriv error reply).
    Notice(String),
    /// Connection failed or dropped.
    Failed(String),
}

/// A pseudo-random draw in `[0, 1)` for jitter (no crate needed for this).
fn jitter_draw() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (nanos % 1_000_000) as f64 / 1_000_000.0
}

pub async fn run(hub: FeedHub, start: UpstreamStart, plan: LivePlan) {
    let UpstreamStart {
        key,
        epoch,
        mut stop,
    } = start;
    let mut backoff = Backoff::default();
    let display = key.source.display();
    loop {
        let mut delivered = false;
        let end = tokio::select! {
            _ = &mut stop => return,
            end = session(&hub, &key, epoch, &plan, &mut delivered) => end,
        };
        if !hub.lock().alive(&key, epoch) {
            return;
        }
        if delivered {
            backoff.reset();
        }
        let (notice, wait, reconnect) = match end {
            End::Notice(status) => (status, plan.end_session_wait, false),
            End::Failed(e) => (
                format!("{display} reconnecting: {e}"),
                backoff.next(jitter_draw()),
                true,
            ),
        };
        tracing::warn!(target: "feed", src = %key.source, every = 30, "{key}: {notice}");
        let f = frame(&ServerMsg::Status { msg: notice });
        for tx in hub.lock().link_down(&key, epoch, f.clone(), reconnect) {
            let _ = tx.send(f.clone());
        }
        tokio::select! {
            _ = &mut stop => return,
            _ = tokio::time::sleep(wait) => {}
        }
    }
}

async fn session(
    hub: &FeedHub,
    key: &sources::UpstreamKey,
    epoch: u64,
    plan: &LivePlan,
    delivered: &mut bool,
) -> End {
    let connect = tokio::time::timeout(
        Duration::from_secs(25),
        tokio_tungstenite::connect_async(plan.url.as_str()),
    );
    let mut ws = match connect.await {
        Ok(Ok((ws, _))) => ws,
        Ok(Err(e)) => return End::Failed(e.to_string()),
        Err(_) => return End::Failed("timed out during opening handshake".into()),
    };
    for hello in &plan.hello {
        if let Err(e) = ws.send(Message::Text(hello.clone().into())).await {
            return End::Failed(e.to_string());
        }
    }
    let ping_every = match plan.keepalive {
        KeepAlive::JsonPing { every } => Some(every),
        KeepAlive::Protocol => None,
    };
    let mut next_ping = ping_every.map(|d| Instant::now() + d);
    loop {
        let wait_ping = async {
            match next_ping {
                Some(t) => tokio::time::sleep_until(t).await,
                None => std::future::pending::<()>().await,
            }
        };
        let msg = tokio::select! {
            _ = wait_ping => {
                if let Err(e) = ws.send(Message::Text(r#"{"op":"ping"}"#.into())).await {
                    return End::Failed(e.to_string());
                }
                next_ping = ping_every.map(|d| Instant::now() + d);
                continue;
            }
            m = ws.next() => m,
        };
        let text = match msg {
            Some(Ok(Message::Text(t))) => t,
            Some(Ok(Message::Close(c))) => {
                return End::Failed(
                    c.map(|c| format!("received {} ({})", u16::from(c.code), c.reason))
                        .unwrap_or_else(|| "connection closed".into()),
                )
            }
            Some(Ok(_)) => continue,
            Some(Err(e)) => return End::Failed(e.to_string()),
            None => return End::Failed("connection closed".into()),
        };
        match sources::parse_frame(key, &text) {
            Parsed::Ignore => {}
            Parsed::EndSession { status } => return End::Notice(status),
            Parsed::Batch(batch) => {
                *delivered = true;
                let clock = hub.clock();
                let emits = hub.lock().ingest(
                    key,
                    epoch,
                    &batch,
                    Instant::now(),
                    clock.true_now_ms(),
                    clock.local_now_ms(),
                );
                for e in emits {
                    let _ = e.tx.send(frame(&e.msg));
                }
            }
        }
    }
}
