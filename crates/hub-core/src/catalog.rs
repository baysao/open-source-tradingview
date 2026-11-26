//! Symbol lists and everything derived from them: `/api/symbols`, `/api/status`, `/api/instruments`.
//!
//! One writer per list (its refresh loop), many readers. Each list is replaced whole, and the
//! instruments are rebuilt at the same moment, so `/api/instruments` can never be stale against
//! `/api/symbols` (Python cached it by total symbol count only).

use crate::market::instruments;
use crate::sources::{binance, Sources};
use hub_types::market::{
    FeedStatus, InstrumentsResponse, Off, StatusResponse, SymbolEntry, SymbolsResponse,
};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::task::JoinHandle;

#[derive(Clone)]
pub struct Catalog {
    inner: Arc<Inner>,
}

struct Inner {
    ws_port: u16,
    binance: Vec<SymbolEntry>,
    lists: RwLock<Lists>,
}

struct Lists {
    deriv: Arc<Vec<SymbolEntry>>,
    bybit: Arc<Vec<SymbolEntry>>,
    instruments: Arc<InstrumentsResponse>,
}

impl Catalog {
    /// An empty catalog; lists fill in as the refresh loops succeed (the server answers before
    /// they do, as Python did: early calls see empty lists and `ok: false`).
    pub fn new(ws_port: u16) -> Catalog {
        let binance = binance::crypto_symbols();
        let instruments = Arc::new(instruments::build(&[], &[], &binance));
        Catalog {
            inner: Arc::new(Inner {
                ws_port,
                binance,
                lists: RwLock::new(Lists {
                    deriv: Arc::new(Vec::new()),
                    bybit: Arc::new(Vec::new()),
                    instruments,
                }),
            }),
        }
    }

    fn lists(
        &self,
    ) -> (
        Arc<Vec<SymbolEntry>>,
        Arc<Vec<SymbolEntry>>,
        Arc<InstrumentsResponse>,
    ) {
        let l = self.inner.lists.read().unwrap_or_else(|p| p.into_inner());
        (l.deriv.clone(), l.bybit.clone(), l.instruments.clone())
    }

    /// `GET /api/symbols`: Bybit, Deriv, then Binance (`hub/server.py:904-908`); `mt5` and `arm`
    /// are "off" (D-003); `ws_port` is this server's port, since `/ws` shares it.
    pub fn symbols(&self) -> SymbolsResponse {
        let (deriv, bybit, _) = self.lists();
        let mut symbols = Vec::with_capacity(bybit.len() + deriv.len() + self.inner.binance.len());
        symbols.extend(bybit.iter().cloned());
        symbols.extend(deriv.iter().cloned());
        symbols.extend(self.inner.binance.iter().cloned());
        SymbolsResponse {
            symbols,
            mt5: Off,
            arm: Off,
            deriv: deriv.len(),
            bybit: bybit.len(),
            ws_port: self.inner.ws_port,
        }
    }

    /// `symbols()` with the session owner's bridge symbols appended (07-03, D-032a/b). The
    /// anonymous body is `symbols()` untouched (G1); each extra entry is the bridge's cached
    /// `/symbols` row verbatim plus `source: "mt5"` and the owning bridge's id. The caller
    /// (the route) reads the cache, never the bridge live.
    pub fn symbols_with(&self, extra: Vec<serde_json::Value>) -> serde_json::Value {
        let base = self.symbols();
        let mut v = serde_json::to_value(&base).expect("SymbolsResponse serializes");
        if let Some(arr) = v.get_mut("symbols").and_then(|s| s.as_array_mut()) {
            arr.extend(extra);
        }
        v
    }

    /// `GET /api/status`: `ok` means the symbol list loaded; Binance is always ok.
    pub fn status(&self) -> StatusResponse {
        let (deriv, bybit, _) = self.lists();
        StatusResponse {
            deriv: FeedStatus {
                ok: !deriv.is_empty(),
                msg: format!("{} symbols", deriv.len()),
            },
            bybit: FeedStatus {
                ok: !bybit.is_empty(),
                msg: format!("{} charts", bybit.len()),
            },
            binance: FeedStatus {
                ok: true,
                msg: "public feed".to_string(),
            },
        }
    }

    /// `GET /api/instruments`.
    pub fn instruments(&self) -> Arc<InstrumentsResponse> {
        self.lists().2
    }

    pub fn set_deriv(&self, list: Vec<SymbolEntry>) {
        self.replace(|l| l.deriv = Arc::new(list));
    }

    pub fn set_bybit(&self, list: Vec<SymbolEntry>) {
        self.replace(|l| l.bybit = Arc::new(list));
    }

    fn replace(&self, f: impl FnOnce(&mut Lists)) {
        let mut l = self.inner.lists.write().unwrap_or_else(|p| p.into_inner());
        f(&mut l);
        l.instruments = Arc::new(instruments::build(&l.deriv, &l.bybit, &self.inner.binance));
    }

    /// The two refresh loops: Bybit every 3,600 s (60 s after a failure); Deriv every 60 s until
    /// its first success, then never again (Python parity).
    pub fn spawn_refresh(&self, sources: Arc<Sources>) -> Vec<JoinHandle<()>> {
        let bybit = {
            let cat = self.clone();
            let src = sources.clone();
            tokio::spawn(async move {
                loop {
                    let wait = match src.bybit_symbols().await {
                        Ok(list) => {
                            tracing::info!(target: "server", "Bybit: {} charts loaded", list.len());
                            cat.set_bybit(list);
                            3600
                        }
                        Err(e) => {
                            tracing::warn!(target: "server", "Bybit symbols failed: {e}");
                            60
                        }
                    };
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                }
            })
        };
        let deriv = {
            let cat = self.clone();
            tokio::spawn(async move {
                loop {
                    match sources.deriv_symbols().await {
                        Ok(list) => {
                            tracing::info!(target: "server", "Deriv: {} symbols loaded", list.len());
                            cat.set_deriv(list);
                            return;
                        }
                        Err(e) => tracing::warn!(target: "server", "Deriv symbols failed: {e}"),
                    }
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
            })
        };
        vec![bybit, deriv]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_with_an_empty_extra_is_byte_identical() {
        let c = Catalog::new(8770);
        assert_eq!(
            serde_json::to_string(&c.symbols()).unwrap(),
            serde_json::to_string(&c.symbols_with(Vec::new())).unwrap(),
            "the anonymous shape is unchanged (G1)"
        );
        // The merged shape still answers the response-level fields (mt5 stays "off", D-003).
        let merged = c.symbols_with(vec![serde_json::json!({
            "name": "XAUUSD", "desc": "Gold", "path": "Other", "digits": 3,
            "source": "mt5", "bridge": uuid::Uuid::nil().to_string(),
        })]);
        assert_eq!(merged["mt5"], "off");
        assert_eq!(merged["arm"], "off");
        let arr = merged["symbols"].as_array().unwrap();
        let last = arr.last().unwrap();
        assert_eq!(last["source"], "mt5");
        assert_eq!(last["bridge"], uuid::Uuid::nil().to_string());
    }
}
