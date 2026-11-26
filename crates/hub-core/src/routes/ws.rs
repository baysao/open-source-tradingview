//! `/ws`: today's protocol on the HTTP port (D-002).
//!
//! Per text message: parse JSON (a parse error closes the socket, as in Python), abort the previous
//! stream, validate the timeframe (an unknown one gets `error "Unknown timeframe X"` and the socket
//! stays open), then stream `history`, the link notice if any, and live frames.

use crate::auth::Viewer;
use crate::bridges::crypto;
use crate::bridges::guard::vet_url;
use crate::bridges::store;
use crate::feeds::bridge::{BridgeId, PollerKey};
use crate::feeds::{SocketPermit, Wanted};
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
use uuid::Uuid;

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
        self.source == o.source
            && self.symbol == o.symbol
            && self.tf == o.tf
            && self.bridge == o.bridge
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
    // D-032a: the optional `bridge` field. Absent, null or empty is `None` — every pre-07-03
    // subscribe, bit for bit. A non-string value is malformed and closes the socket; the
    // string itself is validated at subscribe time, where the state can check it.
    let bridge = match v.get("bridge") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) if s.is_empty() => None,
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(_) => return Parsed::Close,
    };
    Parsed::Subscribe(Wanted {
        source: Source::from_wire(Some(&source_raw)),
        symbol: field("symbol", "XAUUSD"),
        tf,
        bridge,
    })
}

pub async fn upgrade(
    State(st): State<AppState>,
    client: ClientIp,
    ws: WebSocketUpgrade,
    viewer: Option<axum::Extension<Viewer>>,
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
    // The session's user id, for the bridge wants' owner check (07-03). Anonymous sockets keep
    // every pre-07-03 capability.
    let user = viewer.as_ref().and_then(|e| match &e.0 {
        Viewer::Member(m) => Some(m.user.id),
        Viewer::Anonymous => None,
    });
    ws.max_message_size(4096)
        .on_upgrade(move |socket| serve_socket(socket, st, user, permit))
}

async fn serve_socket(socket: WebSocket, st: AppState, user: Option<Uuid>, _permit: SocketPermit) {
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
                if want.bridge.is_some() {
                    // The bridge path (07-03, D-032a/c): the owner check and the credential
                    // resolution run inline — a foreign, unknown or malformed id sends the
                    // error frame and closes the socket, matching the ws error style.
                    match bridge_stream(&st, user, &want).await {
                        Ok(sub) => {
                            let out = out_tx.clone();
                            current = Some(tokio::spawn(async move {
                                let mut sub = sub;
                                while let Some(f) = sub.next().await {
                                    if out.send(Message::Text(f)).await.is_err() {
                                        return;
                                    }
                                }
                            }));
                        }
                        Err(msg) => {
                            let _ = out_tx
                                .send(Message::Text(crate::feeds::hub::frame(&ServerMsg::Error {
                                    msg,
                                    kind: None,
                                })))
                                .await;
                            break;
                        }
                    }
                } else {
                    let (hub, out) = (st.feeds.clone(), out_tx.clone());
                    current = Some(tokio::spawn(async move {
                        match hub.subscribe(want).await {
                            Err(refusal) => {
                                let _ = out
                                    .send(Message::Text(crate::feeds::hub::frame(
                                        &refusal.message(),
                                    )))
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
    }
    if let Some(h) = current.take() {
        h.abort();
    }
    drop(out_tx);
    let _ = writer.await;
}

/// The bridge subscription (07-03, D-032a/c): resolve the want once — the owner check at
/// subscribe time, the URL re-vetted and pinned exactly as the REST path vets — then join or
/// start the shared poller for `(bridge, symbol, tf)` (one per want across the user's open
/// charts; the last unsubscribe stops it). `Err` carries the error frame's text; the caller
/// closes the socket after sending it.
async fn bridge_stream(
    st: &AppState,
    user: Option<Uuid>,
    want: &Wanted,
) -> Result<crate::feeds::bridge::BridgeSub, String> {
    let raw = want.bridge.as_deref().unwrap_or("");
    let (client_bridge, vetted, token, key) = if raw == "local" {
        // The env-configured local bridge (D-033): auth off only, loopback only (the guard's
        // auth-off rule is the seam). The vet pins the addresses the poller will dial.
        let local = match (&st.bridges.local, st.cfg.auth.is_local()) {
            (Some(l), true) => l,
            _ => return Err("unauthorized".into()),
        };
        let vetted = vet_url(&local.url, false)
            .await
            .map_err(|e| e.to_string())?;
        let key = PollerKey {
            bridge: BridgeId::Local,
            symbol: want.symbol.clone(),
            tf: want.tf,
        };
        (
            crate::bridges::LOCAL_SEMAPHORE_ID,
            vetted,
            local.token.expose().to_string(),
            key,
        )
    } else {
        let Ok(id) = Uuid::parse_str(raw) else {
            return Err("unauthorized".into());
        };
        let uid = user.ok_or_else(|| "unauthorized".to_string())?;
        let auth = st.auth.as_ref().ok_or_else(|| "unauthorized".to_string())?;
        let row = store::get_owned(auth.bridge_pool(), uid, id)
            .await
            .map_err(|e| {
                tracing::warn!(target: "server", "ws bridge lookup: {e}");
                "service unavailable".to_string()
            })?
            .ok_or_else(|| "unauthorized".to_string())?;
        let token = crypto::open(&super::mt5::bridge_key(st), &row.token_enc)
            .map_err(|_| "unauthorized".to_string())?;
        let vetted = vet_url(&row.url, true).await.map_err(|e| e.to_string())?;
        let key = PollerKey {
            bridge: BridgeId::Named(id),
            symbol: want.symbol.clone(),
            tf: want.tf,
        };
        (id, vetted, token, key)
    };
    let symbol = key.symbol.clone();
    let tf = key.tf;
    let client = st.bridges.client.clone();
    Ok(st.bridge_pollers.subscribe(key, |tx| {
        tokio::spawn(crate::feeds::bridge::run_poller(
            tx,
            client,
            client_bridge,
            vetted,
            token,
            symbol,
            tf,
        ))
    }))
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
                tf: Tf::M1,
                bridge: None
            })
        );
        // D-009 parity: source=mt5 without a bridge keeps the Binance fallback.
        assert_eq!(
            parse_subscribe(r#"{"source":"mt5","symbol":"BTCUSD","tf":"H1"}"#),
            Parsed::Subscribe(Wanted {
                source: Source::Binance,
                symbol: "BTCUSD".into(),
                tf: Tf::H1,
                bridge: None
            })
        );
    }

    #[test]
    fn bridge_field_variants() {
        // Absent, null and empty are `None`; a string rides through verbatim; a non-string
        // value is malformed and closes the socket.
        let with = |s: &str| {
            parse_subscribe(&format!(
                r#"{{"source":"mt5","bridge":{s},"symbol":"X","tf":"H1"}}"#
            ))
        };
        let id = uuid::Uuid::now_v7();
        assert_eq!(
            with(&format!(r#""{id}""#)),
            Parsed::Subscribe(Wanted {
                source: Source::Binance,
                symbol: "X".into(),
                tf: Tf::H1,
                bridge: Some(id.to_string())
            })
        );
        assert_eq!(
            with("null"),
            Parsed::Subscribe(Wanted {
                source: Source::Binance,
                symbol: "X".into(),
                tf: Tf::H1,
                bridge: None
            })
        );
        assert_eq!(
            with(r#""""#),
            Parsed::Subscribe(Wanted {
                source: Source::Binance,
                symbol: "X".into(),
                tf: Tf::H1,
                bridge: None
            })
        );
        assert_eq!(with("5"), Parsed::Close);
    }
}
