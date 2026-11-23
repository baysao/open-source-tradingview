//! `/ws`: today's protocol on the HTTP port (D-002).
//!
//! Per text message: parse JSON (a parse error closes the socket, as in Python), abort the previous
//! stream, validate the timeframe (an unknown one gets `error "Unknown timeframe X"` and the socket
//! stays open), then stream `history`, the link notice if any, and live frames.

use crate::feeds::{FeedHub, SocketPermit, Wanted};
use crate::net::ClientIp;
use crate::routes::api_error;
use crate::AppState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use hub_types::market::{Source, Tf};
use hub_types::ws::ServerMsg;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;

const PING_EVERY: Duration = Duration::from_secs(30);
const PONG_DEADLINE: Duration = Duration::from_secs(60);

/// The outcome of one browser text message. Pure.
#[derive(Debug, PartialEq)]
pub enum Parsed {
    /// Not JSON: close the socket (Python's `json.loads` raised and ended the handler).
    Close,
    /// Bad timeframe: send this error, keep the socket.
    Reject(ServerMsg),
    Subscribe(Wanted),
}

impl PartialEq for Wanted {
    fn eq(&self, o: &Wanted) -> bool {
        self.source == o.source && self.symbol == o.symbol && self.tf == o.tf
    }
}

/// Defaults `binance`, `XAUUSD`, `M1`; an unknown source means Binance.
pub fn parse_subscribe(text: &str) -> Parsed {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return Parsed::Close;
    };
    let field = |k: &str, d: &str| -> String {
        match v.get(k) {
            None | Some(serde_json::Value::Null) => d.to_string(),
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
        }
    };
    let tf_raw = field("tf", "M1");
    let Some(tf) = Tf::parse(&tf_raw) else {
        return Parsed::Reject(ServerMsg::Error {
            msg: format!("Unknown timeframe {tf_raw}"),
            kind: None,
        });
    };
    let source_raw = field("source", "binance");
    Parsed::Subscribe(Wanted {
        source: Source::from_wire(Some(&source_raw)),
        symbol: field("symbol", "XAUUSD"),
        tf,
    })
}

pub async fn upgrade(
    State(st): State<AppState>,
    client: ClientIp,
    ws: WebSocketUpgrade,
) -> Response {
    let permit = match st.feeds.admit_socket(client.ip, client.peer_loopback) {
        Ok(p) => p,
        Err(_) => {
            return api_error(
                StatusCode::TOO_MANY_REQUESTS,
                "too many chart connections from this address",
            )
        }
    };
    let feeds = st.feeds.clone();
    ws.max_message_size(4096)
        .on_upgrade(move |socket| serve_socket(socket, feeds, permit))
}

async fn serve_socket(socket: WebSocket, feeds: FeedHub, _permit: SocketPermit) {
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::channel::<Message>(512);
    let writer = tokio::spawn(async move {
        while let Some(m) = out_rx.recv().await {
            if sink.send(m).await.is_err() {
                break;
            }
        }
    });
    let mut current: Option<tokio::task::JoinHandle<()>> = None;
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    let mut last_pong = Instant::now();
    loop {
        let msg = tokio::select! {
            _ = ping.tick() => {
                if last_pong.elapsed() > PONG_DEADLINE {
                    break;
                }
                if out_tx.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
                continue;
            }
            m = stream.next() => m,
        };
        let text = match msg {
            Some(Ok(Message::Text(t))) => t,
            Some(Ok(Message::Pong(_))) => {
                last_pong = Instant::now();
                continue;
            }
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
            Some(Ok(_)) => continue,
        };
        match parse_subscribe(&text) {
            Parsed::Close => break,
            Parsed::Reject(err) => {
                if let Some(h) = current.take() {
                    h.abort();
                }
                let _ = out_tx
                    .send(Message::Text(crate::feeds::hub::frame(&err)))
                    .await;
            }
            Parsed::Subscribe(want) => {
                if let Some(h) = current.take() {
                    h.abort();
                }
                let (hub, out) = (feeds.clone(), out_tx.clone());
                current = Some(tokio::spawn(async move {
                    match hub.subscribe(want).await {
                        Err(refusal) => {
                            let _ = out
                                .send(Message::Text(crate::feeds::hub::frame(&refusal.message())))
                                .await;
                        }
                        Ok(mut sub) => {
                            while let Some(f) = sub.next().await {
                                if out.send(Message::Text(f)).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }));
            }
        }
    }
    if let Some(h) = current.take() {
        h.abort();
    }
    drop(out_tx);
    let _ = writer.await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_parsing_matches_python() {
        assert_eq!(parse_subscribe("not json"), Parsed::Close);
        assert_eq!(
            parse_subscribe(r#"{"tf":"M2"}"#),
            Parsed::Reject(ServerMsg::Error {
                msg: "Unknown timeframe M2".into(),
                kind: None
            })
        );
        assert_eq!(
            parse_subscribe("{}"),
            Parsed::Subscribe(Wanted {
                source: Source::Binance,
                symbol: "XAUUSD".into(),
                tf: Tf::M1
            })
        );
        assert_eq!(
            parse_subscribe(r#"{"source":"mt5","symbol":"BTCUSD","tf":"H1"}"#),
            Parsed::Subscribe(Wanted {
                source: Source::Binance,
                symbol: "BTCUSD".into(),
                tf: Tf::H1
            })
        );
    }
}
